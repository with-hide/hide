// A small picture of a phone screen that a What's new scene or a practice chapter attaches as a photo (Doop 08, 09). It is drawn
// in the theme's own colors, read from the page's tokens when it is made, so it is never a picture of anyone's screen and it follows
// light and dark like everything around it.

const ROWS = [
  { y: 62, width: 84, accent: false },
  { y: 78, width: 64, accent: false },
  { y: 102, width: 90, accent: true },
  { y: 118, width: 56, accent: false },
  { y: 142, width: 78, accent: false },
  { y: 158, width: 48, accent: false },
] as const;

/** The picture as a data URL, for an `img` of the phone's attachment parts. */
export function sampleShot(): string {
  const style = typeof document === "undefined" ? null : getComputedStyle(document.documentElement);
  const color = (name: string) => style?.getPropertyValue(name).trim() || "currentColor";
  const card = color("--color-card");
  const line = color("--color-border");
  const text = color("--color-foreground");
  const muted = color("--color-muted-foreground");
  const accent = color("--color-agent-working");
  const bar = (y: number, width: number, fill: string, height = 6) => `<rect x="12" y="${y}" width="${width}" height="${height}" rx="3" style="fill:${fill}"/>`;
  const svg = [
    `<svg xmlns="http://www.w3.org/2000/svg" width="120" height="200" viewBox="0 0 120 200">`,
    `<rect width="120" height="200" style="fill:${card}"/>`,
    bar(16, 58, text, 8),
    bar(32, 36, muted),
    `<rect y="48" width="120" height="1" style="fill:${line}"/>`,
    ...ROWS.map((row) => bar(row.y, row.width, row.accent ? accent : muted)),
    `</svg>`,
  ].join("");
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}
