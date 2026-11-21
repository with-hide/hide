// What Markdown Live is willing to draw from a document's HTML and images
// (PRD markdown-html): the sanitizer for an HTML block or tag, and the
// decision of where an image source may be read from. Both are the security
// boundary of the feature, so they sit apart from the widgets that draw.
//
// HTML is cleaned by DOMPurify on an allowlist of presentational tags and
// attributes: no script, style, iframe, object, form, event handler, inline
// style or script URL survives, and a link keeps only http(s), mailto, a
// fragment or a relative path. An image's `src` is left for `imageTarget`,
// because the same document may name a file, a web address or inline bytes
// and each has its own rule.

import DOMPurify from "dompurify";

const TAGS = [
  "p", "div", "span", "a", "img", "br", "hr",
  "h1", "h2", "h3", "h4", "h5", "h6",
  "strong", "b", "em", "i", "u", "s", "strike", "del", "ins", "mark", "small", "sub", "sup",
  "code", "pre", "kbd", "samp", "abbr", "cite", "q", "blockquote",
  "ul", "ol", "li", "dl", "dt", "dd",
  "table", "thead", "tbody", "tfoot", "tr", "th", "td", "caption", "colgroup", "col",
  "details", "summary", "figure", "figcaption",
];

// `align` and `valign` are the presentational attributes READMEs centre a logo
// with; `width` and `height` size an image or a table cell.
const ATTRIBUTES = ["href", "src", "alt", "title", "width", "height", "align", "valign", "colspan", "rowspan", "open"];

// Anything else with a scheme (`javascript:`, `vbscript:`, `file:`, `ftp:`) is
// dropped; a path with no scheme and a `#fragment` stay.
const LINK_URI = /^(?:(?:https?|mailto):|[^a-z]|[a-z][a-z0-9+.-]*(?:[^a-z0-9+.:-]|$))/i;

/**
 * The address a link drawn by Live may carry, the same rule the sanitizer
 * keeps an `href` by: http(s), `mailto:`, a fragment or a relative path, which
 * a modifier-click then opens (`openLink`); null for any other scheme.
 */
export function liveLinkAddress(raw: string): string | null {
  const href = raw.trim();
  return LINK_URI.test(href) ? href : null;
}

let purifier: ReturnType<typeof DOMPurify> | null = null;

function instance() {
  purifier ??= DOMPurify(window);
  return purifier;
}

/**
 * The sanitized fragment for `html`, or null when nothing visible is left of
 * it (a comment, an empty element), in which case the source stays on screen.
 * An `<img>`'s `src` is removed from the fragment and returned in `images`
 * beside the element, so nothing is fetched before `imageTarget` has decided.
 */
export function sanitizeHtml(html: string): { fragment: DocumentFragment; images: { element: HTMLImageElement; src: string }[] } | null {
  const fragment = instance().sanitize(html, {
    ALLOWED_TAGS: TAGS,
    ALLOWED_ATTR: ATTRIBUTES,
    ALLOWED_URI_REGEXP: LINK_URI,
    ALLOW_DATA_ATTR: false,
    ALLOW_ARIA_ATTR: false,
    RETURN_DOM_FRAGMENT: true,
  });
  const images: { element: HTMLImageElement; src: string }[] = [];
  for (const element of Array.from(fragment.querySelectorAll("img"))) {
    const src = element.getAttribute("src") ?? "";
    element.removeAttribute("src");
    images.push({ element, src });
  }
  const visible = (fragment.textContent ?? "").trim() !== "" || fragment.querySelector("img, hr") !== null;
  return visible ? { fragment, images } : null;
}

const drawable = new Map<string, boolean>();
const DRAWABLE_CAP = 512;

/**
 * Whether `html` leaves something to draw once cleaned. A block that leaves
 * nothing (a bare `<div align="center">`, a `</div>`, a comment) stays source,
 * so this is asked while the decorations are planned; the answer is kept by
 * source text so a keystroke elsewhere does not clean the same block again.
 */
export function hasVisibleHtml(html: string): boolean {
  const known = drawable.get(html);
  if (known !== undefined) return known;
  if (drawable.size >= DRAWABLE_CAP) drawable.clear();
  const visible = sanitizeHtml(html) !== null;
  drawable.set(html, visible);
  return visible;
}

/** Where an image may be read from, or why it may not be. */
export type ImageTarget =
  | { kind: "file"; path: string; mime: string }
  | { kind: "remote"; url: string }
  | { kind: "data"; url: string }
  | { kind: "refused"; reason: "empty" | "scheme" | "type" | "no_base" | "outside_checkout" | "malformed" };

const MIME: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  avif: "image/avif",
  bmp: "image/bmp",
  svg: "image/svg+xml",
};

/**
 * `data:image/*` is read because it names its own bytes: it opens no
 * connection and reads no file, and an `<img>` runs no script from an SVG.
 * It counts against the same byte cap as a file.
 */
const DATA_IMAGE = /^data:image\/(?:png|jpe?g|gif|webp|avif|bmp|svg\+xml)[;,]/i;

export type ImageBase = { docPath: string; root: string };

/**
 * Normalizes a `/`-separated path, resolving `.` and `..`; null when `..`
 * climbs above the first segment.
 */
function normalize(path: string): string | null {
  const parts: string[] = [];
  for (const segment of path.split("/")) {
    if (segment === "." || (segment === "" && parts.length > 0)) continue;
    if (segment === "..") {
      if (parts.length <= 1) return null;
      parts.pop();
      continue;
    }
    parts.push(segment);
  }
  return parts.join("/");
}

/**
 * A relative reference resolved against the document's folder, or against the
 * checkout root when it starts with `/` the way a repository-relative link
 * does; null when the result leaves the checkout. A `?query` or `#fragment` is
 * dropped and percent escapes are decoded.
 */
export function resolveInCheckout(reference: string, base: ImageBase): string | null {
  const bare = reference.split(/[?#]/)[0] ?? "";
  if (bare === "") return null;
  let decoded: string;
  try {
    decoded = decodeURIComponent(bare);
  } catch {
    return null;
  }
  // A backslash is a separator on a Windows host, and `normalize` only knows
  // `/`, so `..\..\x` would climb out there; a control character names nothing.
  // eslint-disable-next-line no-control-regex
  if (/[\\\u0000-\u001f]/.test(decoded)) return null;
  const root = base.root.replace(/\/+$/, "");
  const folder = base.docPath.slice(0, base.docPath.lastIndexOf("/"));
  const joined = decoded.startsWith("/") ? `${root}${decoded}` : `${folder}/${decoded}`;
  const resolved = normalize(joined);
  return resolved !== null && resolved.startsWith(`${root}/`) ? resolved : null;
}

export function imageTarget(src: string, base: ImageBase | null): ImageTarget {
  const trimmed = src.trim();
  if (trimmed === "") return { kind: "refused", reason: "empty" };
  const scheme = /^([a-z][a-z0-9+.-]*):/i.exec(trimmed)?.[1]?.toLowerCase();
  if (scheme === "https") {
    try {
      const url = new URL(trimmed);
      return url.hostname === "" ? { kind: "refused", reason: "malformed" } : { kind: "remote", url: url.href };
    } catch {
      return { kind: "refused", reason: "malformed" };
    }
  }
  if (scheme === "data") return DATA_IMAGE.test(trimmed) ? { kind: "data", url: trimmed } : { kind: "refused", reason: "type" };
  if (scheme !== undefined || trimmed.startsWith("//")) return { kind: "refused", reason: "scheme" };
  if (!base) return { kind: "refused", reason: "no_base" };
  const path = resolveInCheckout(trimmed, base);
  if (path === null) return { kind: "refused", reason: "outside_checkout" };
  const mime = MIME[path.slice(path.lastIndexOf(".") + 1).toLowerCase()];
  return mime ? { kind: "file", path, mime } : { kind: "refused", reason: "type" };
}
