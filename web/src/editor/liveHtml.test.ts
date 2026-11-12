// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { hasVisibleHtml, imageTarget, liveLinkAddress, resolveInCheckout, sanitizeHtml } from "./liveHtml";

const base = { docPath: "/work/repo/docs/guide/README.md", root: "/work/repo" };

function clean(html: string): string {
  const result = sanitizeHtml(html);
  if (!result) return "";
  const holder = document.createElement("div");
  holder.append(result.fragment);
  return holder.innerHTML;
}

describe("sanitizeHtml", () => {
  it("keeps the presentational markup a README centres its logo with", () => {
    const result = sanitizeHtml('<p align="center"><img src="logo.svg" width="120" alt="owl"></p>');
    expect(result?.fragment.querySelector("p")?.getAttribute("align")).toBe("center");
    const image = result?.images[0];
    expect(image?.src).toBe("logo.svg");
    // The source is held back so nothing is fetched before the policy runs.
    expect(image?.element.hasAttribute("src")).toBe(false);
    expect(image?.element.getAttribute("width")).toBe("120");
    expect(image?.element.getAttribute("alt")).toBe("owl");
  });

  it("removes script, style, frames, forms, handlers and inline style", () => {
    expect(clean("<p>a</p><script>alert(1)</script>")).toBe("<p>a</p>");
    expect(clean("<style>p{display:none}</style><p>a</p>")).toBe("<p>a</p>");
    expect(clean('<iframe src="https://x"></iframe><object data="x"></object><p>a</p>')).toBe("<p>a</p>");
    expect(clean('<form action="https://x"><input name="q"><button>go</button></form><p>a</p>')).not.toMatch(/form|input|button/);
    expect(clean('<p onclick="alert(1)" style="color:red">a</p>')).toBe("<p>a</p>");
    expect(clean('<img src="x.png" onerror="alert(1)">')).not.toMatch(/onerror/);
  });

  it("keeps a link only when it is http(s), mailto, a fragment or a relative path", () => {
    expect(clean('<a href="https://example.com/x">a</a>')).toContain('href="https://example.com/x"');
    expect(clean('<a href="mailto:a@b.c">a</a>')).toContain("mailto:a@b.c");
    expect(clean('<a href="#install">a</a>')).toContain('href="#install"');
    expect(clean('<a href="docs/README.md">a</a>')).toContain('href="docs/README.md"');
    expect(clean('<a href="javascript:alert(1)">a</a>')).not.toMatch(/href/);
    expect(clean('<a href="JaVaScRiPt:alert(1)">a</a>')).not.toMatch(/href/);
    expect(clean('<a href="file:///etc/passwd">a</a>')).not.toMatch(/href/);
    expect(clean('<a href="s3:bucket/key">a</a>')).not.toMatch(/href/);
    expect(clean('<a href="a1:x">a</a>')).not.toMatch(/href/);
    expect(clean('<a href="vbscript:x">a</a>')).not.toMatch(/href/);
    expect(clean('<a href="data:text/html,<script>alert(1)</script>">a</a>')).not.toMatch(/href/);
  });

  it("leaves the source on screen when nothing visible survives", () => {
    expect(sanitizeHtml("<script>alert(1)</script>")).toBeNull();
    expect(sanitizeHtml("<span></span>")).toBeNull();
  });
});

describe("liveLinkAddress", () => {
  it("keeps the addresses the sanitizer keeps an href by, a relative file among them, and refuses every other scheme", () => {
    for (const kept of ["https://example.com/x", "http://example.com", "mailto:a@b.c", "#install", "docs/README.md", "../notes.md", "/src/x.ts"]) {
      expect(liveLinkAddress(` ${kept} `)).toBe(kept);
    }
    for (const refused of ["javascript:alert(1)", "JaVaScRiPt:alert(1)", "file:///etc/passwd", "data:text/html,x", "vbscript:x", "s3:bucket/key", ""]) {
      expect(liveLinkAddress(refused)).toBeNull();
    }
  });
});

describe("imageTarget", () => {
  it("resolves a relative path against the document's folder and stays inside the checkout", () => {
    expect(imageTarget("shot.png", base)).toEqual({ kind: "file", path: "/work/repo/docs/guide/shot.png", mime: "image/png" });
    expect(imageTarget("../img/a%20b.webp?raw=1#x", base)).toEqual({ kind: "file", path: "/work/repo/docs/img/a b.webp", mime: "image/webp" });
    expect(imageTarget("/site/assets/logo.svg", base)).toEqual({ kind: "file", path: "/work/repo/site/assets/logo.svg", mime: "image/svg+xml" });
  });

  it("refuses a path that leaves the checkout, on either side of the boundary", () => {
    expect(imageTarget("../../../x.png", base)).toEqual({ kind: "refused", reason: "outside_checkout" });
    expect(imageTarget("../../../../etc/x.png", base)).toEqual({ kind: "refused", reason: "outside_checkout" });
    expect(imageTarget("../../../repo-other/x.png", base)).toEqual({ kind: "refused", reason: "outside_checkout" });
    expect(imageTarget("../../a.png", base)).toEqual({ kind: "file", path: "/work/repo/a.png", mime: "image/png" });
  });

  it("refuses a backslash, an encoded one and a control character, which climb out on a Windows host", () => {
    for (const src of ["..\\..\\..\\x.png", "..%5C..%5C..%5Cx.png", "..%5cx.png", "a\\b.png", "a%00.png", "a%0A.png"]) {
      expect(imageTarget(src, base), src).toEqual({ kind: "refused", reason: "outside_checkout" });
    }
    expect(imageTarget("%2e%2e/%2e%2e/%2e%2e/x.png", base)).toEqual({ kind: "refused", reason: "outside_checkout" });
  });

  it("refuses a relative path when the text has no file, and a file that is not an image type", () => {
    expect(imageTarget("a.png", null)).toEqual({ kind: "refused", reason: "no_base" });
    expect(imageTarget("a.html", base)).toEqual({ kind: "refused", reason: "type" });
  });

  it("loads https and image data URIs, and nothing else", () => {
    expect(imageTarget("https://img.shields.io/badge/a-b.svg", base)).toEqual({ kind: "remote", url: "https://img.shields.io/badge/a-b.svg" });
    expect(imageTarget("data:image/png;base64,AAAA", base)).toEqual({ kind: "data", url: "data:image/png;base64,AAAA" });
    expect(imageTarget("data:text/html,<script>", base)).toEqual({ kind: "refused", reason: "type" });
    expect(imageTarget("http://example.com/a.png", base)).toEqual({ kind: "refused", reason: "scheme" });
    expect(imageTarget("file:///etc/passwd", base)).toEqual({ kind: "refused", reason: "scheme" });
    expect(imageTarget("//example.com/a.png", base)).toEqual({ kind: "refused", reason: "scheme" });
    expect(imageTarget("javascript:alert(1)", base)).toEqual({ kind: "refused", reason: "scheme" });
    expect(imageTarget("  ", base)).toEqual({ kind: "refused", reason: "empty" });
  });
});

describe("resolveInCheckout", () => {
  it("names a link target inside the checkout and refuses one outside it", () => {
    expect(resolveInCheckout("../../LICENSE", base)).toBe("/work/repo/LICENSE");
    expect(resolveInCheckout("../../../LICENSE", base)).toBeNull();
    expect(resolveInCheckout("#install", base)).toBeNull();
  });
});

describe("hasVisibleHtml", () => {
  it("is false for a block that cleans to nothing, so it stays source", () => {
    for (const html of ['<div align="center">', "</div>", '<p align="center">', "<br>", "<script>x</script>"]) {
      expect(hasVisibleHtml(html), html).toBe(false);
    }
    expect(hasVisibleHtml("<p>hi</p>")).toBe(true);
    expect(hasVisibleHtml('<img src="a.png">')).toBe(true);
  });
});
