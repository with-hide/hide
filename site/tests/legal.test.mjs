import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// The two pages App Store Connect points at: a privacy policy and a support page.
// They are static files with no build step, so this reads them as text and checks
// the properties a link, a reviewer or a search engine depends on.
const page = name => readFileSync(fileURLToPath(new URL(`../${name}.html`, import.meta.url)), "utf8");
const index = page("index");
const privacy = page("privacy");
const support = page("support");

test("the site footer links both legal pages, and each page links the other", () => {
  for (const href of ["./privacy", "./support"]) {
    assert.match(index, new RegExp(`<footer[\\s\\S]*href="${href.replace(".", "\\.")}"`), `${href} is in the footer`);
  }
  assert.match(privacy, /href="\.\/support"/);
  assert.match(support, /href="\.\/privacy"/);
});

test("each page declares its own canonical URL, title and description", () => {
  for (const [name, html] of [["privacy", privacy], ["support", support]]) {
    assert.match(html, new RegExp(`<link rel="canonical" href="https://withhide\\.dev/${name}">`));
    assert.match(html, /<title>hide · [^<]+<\/title>/);
    assert.match(html, /<meta name="description" content="[^"]{40,}">/);
    assert.match(html, /<meta property="og:title" content="[^"]+">/);
  }
});

test("both pages carry an English and a Korean body, each labelled with its language", () => {
  for (const html of [privacy, support]) {
    assert.match(html, /<section class="legal" id="en" lang="en"/);
    assert.match(html, /<section class="legal" id="ko" lang="ko"/);
    assert.match(html, /<nav class="legal-langs" aria-label="Language">[\s\S]*href="#en"[\s\S]*href="#ko"/);
    // A heading in the Korean body, so the language section is not an empty anchor.
    assert.match(html, /<section class="legal" id="ko" lang="ko"[\s\S]*<h3>[^<]*[가-힣][^<]*<\/h3>/);
  }
});

test("the privacy policy states an effective date, a contact address and the policy sections", () => {
  assert.match(privacy, /<p class="meta">Effective \d{1,2} [A-Z][a-z]+ \d{4}<\/p>/);
  assert.match(privacy, /mailto:hello@withhide\.dev/);
  for (const heading of [
    "The desktop app", "This website", "The relay", "The phone app", "Hide AI",
    "What we never do", "Children", "Your choices and requests", "Changes to this policy", "Contact",
  ]) {
    assert.ok(privacy.includes(`<h3>${heading}</h3>`) || new RegExp(`<h3>\\d+\\. ${heading}</h3>`).test(privacy), `privacy section: ${heading}`);
  }
  // The Korean body carries the same numbered sections.
  assert.equal((privacy.match(/<h3>\d+\. /g) ?? []).length, 20, "ten numbered sections in each language");
});

test("the support page covers what a paired phone needs: connecting, help and removal", () => {
  for (const heading of ["What hide is", "Connect your phone", "Get help", "Remove a paired phone"]) {
    assert.ok(support.includes(`<h3>${heading}</h3>`), `support section: ${heading}`);
  }
  // The ways a person gets help, and the address the site already publishes.
  assert.match(support, /issues\/new\?template=bug\.yml/);
  assert.match(support, /issues\/new\?template=feature_request\.yml/);
  assert.match(support, /security\/advisories\/new/);
  assert.match(support, /mailto:hello@withhide\.dev/);
});

test("no page loads a third-party script, so the policy's claim about the site stays true", () => {
  for (const html of [index, privacy, support]) {
    const sources = [...html.matchAll(/<script[^>]*\bsrc="([^"]+)"/g)].map(m => m[1]);
    for (const src of sources) {
      assert.ok(
        src === "/_vercel/insights/script.js" || src.startsWith("./"),
        `only this site's own scripts and Vercel's analytics are loaded, found ${src}`,
      );
    }
  }
});

test("the pages the policy links live on this site point at a page that exists", () => {
  for (const [name, html] of [["privacy", privacy], ["support", support]]) {
    for (const [, href] of html.matchAll(/href="\.\/([a-z-]+)"/g)) {
      assert.ok(["privacy", "support", ""].includes(href), `${name} links only to this site's pages, found ./${href}`);
    }
  }
});
