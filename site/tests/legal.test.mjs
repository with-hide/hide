import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// The two pages App Store Connect points at: a privacy policy and a support page.
// They are static files with no build step, so this reads them as text and checks
// the properties a link, a reviewer or a search engine depends on.
const page = name => readFileSync(fileURLToPath(new URL(`../public/${name}.html`, import.meta.url)), "utf8");
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
  // The iOS app was released on 13 November 2026, and the policy states that day.
  assert.match(privacy, /<p class="meta">Effective (\d{1,2} [A-Z][a-z]+ \d{4}|from the day the iOS app is released on the App Store)<\/p>/);
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

// The Android app is not on Google Play yet, so no page offers a download link that would answer "not found".
// The privacy policy still names Google Play services, which the app needs; it never links the store.
test("no site page links Google Play until the Android app is published, and the pages say it is coming", () => {
  const faq = repoFile("site/src/content/docs/docs/reference/faq.md");
  for (const [name, text] of [["index", index], ["privacy", privacy], ["support", support], ["faq", faq]]) {
    assert.doesNotMatch(text, /play\.google\.com/, `${name} does not link Google Play`);
  }
  assert.match(index, /Android app coming to Google Play/);
  assert.match(support, /The Android app is coming to Google Play\./);
  assert.match(support, /Android 앱은 Google Play에 곧 올라옵니다/);
  assert.match(faq, /The Android app is coming to Google Play\./);
});

// The product's own files are the expected answer here: the event names in the Rust
// source and the retention in the deploy files. The page is the public statement of
// both, so it fails the build when it falls behind them.
const repoFile = rel => readFileSync(fileURLToPath(new URL(`../../${rel}`, import.meta.url)), "utf8");
const section = (html, id) => html.match(new RegExp(`<section class="legal" id="${id}"[\\s\\S]*?</section>`))[0];

test("the usage-statistics table has one row per event hide-core sends, in each language", () => {
  const source = repoFile("hide-core/src/product_analytics.rs");
  const names = [...source.match(/pub fn name\(self\)[\s\S]*?\n    }\n/)[0].matchAll(/=> "(hide_[a-z_]+)"/g)].map(m => m[1]).sort();
  assert.ok(names.length >= 7, `read the event names from Event::name, found ${names}`);
  for (const id of ["en", "ko"]) {
    const rows = [...section(privacy, id).matchAll(/<tr data-event="([a-z_]+)">/g)].map(m => m[1]).sort();
    assert.deepEqual(rows, names, `${id}: the policy's event table matches Event::name in hide-core/src/product_analytics.rs`);
  }
});

test("the docs that own the events and the relay logs name the page that states them", () => {
  assert.match(repoFile("docs/ARCHITECTURE.md"), /site\/public\/privacy\.html/);
  assert.match(repoFile("docs/RELAY_PROTOCOL.md"), /site\/public\/privacy\.html/);
});

test("the policy's relay journal retention is not shorter than what the deploy keeps", () => {
  const vacuum = repoFile("deploy/relay/hide-relay-retention.service").match(/--vacuum-time=(\d+)d/);
  assert.ok(vacuum, "the deploy vacuums the journal by days");
  // Whole files are vacuumed after an hourly rotation, so a line can outlive the vacuum
  // age by up to a day; the page promises the day count plus that one.
  assert.equal(Number(vacuum[1]) + 1, 7, "the page says seven days");
  assert.equal((privacy.match(/at most seven days/g) ?? []).length, 1);
  assert.equal((privacy.match(/최대 7일/g) ?? []).length, 1);
});

test("the policy names what Google sees through FCM and that the relay passes the token, in each language", () => {
  // The Android app's notifications go through Google's FCM; the Play data safety form declares the same facts (mobile/store/android/play-forms.md).
  const claims = {
    en: [
      /passes on to Google \(section 4\.4\)/, // 3.1, what the relay sees
      /<h4>4\.4 The Android app and Google<\/h4>/, /in neither can Google read anything that comes from your computer/,
      /Firebase Cloud Messaging \(FCM\)/, /Firebase installation ID/, /The relay passes both to Google/, /does not store the token/,
      /when Google reports the token as no longer valid/, /system version, model, brand and the store that installed the app/,
      /not even a fixed general line/,
      /code scanner[\s\S]*device identifier and a per-installation identifier, performance figures, and event types and error codes/,
    ],
    ko: [
      /구글로 전달하며/,
      /<h4>4\.4 Android 앱과 구글<\/h4>/, /어느 쪽에서도 구글은 사용자의 컴퓨터에서 온 내용을 읽을 수 없습니다/,
      /Firebase Cloud Messaging\(FCM\)/, /Firebase 설치 ID/, /중계 서버는[^<]*둘을 구글에 전달/, /토큰도 알림도 저장하거나 로그에 남기지 않습니다/,
      /구글이 토큰을 더 이상 쓸 수 없다고 알려 줄 때도 등록이 끝납니다/, /시스템 버전, 모델, 브랜드, 앱을 설치한 스토어/,
      /고정된 일반 문구도 없으므로/,
      /코드 스캐너[\s\S]*기기 식별자와 설치별 식별자, 성능 수치, 이벤트 종류와 오류 코드/,
    ],
  };
  for (const [id, patterns] of Object.entries(claims)) {
    const body = section(privacy, id);
    for (const re of patterns) assert.match(body, re, `${id}: the policy carries ${re}`);
    assert.match(body, /<li>[^<]*FCM[^<]*<\/li>/, `${id}: the relay's list of what it sees names FCM`);
    // The scan session ID is collected only with auto-zoom, which the app leaves off, so the policy does not claim it.
    assert.doesNotMatch(body, /session ID|세션 ID/, `${id}: no scan session ID`);
  }
});

test("the Play data safety answers say what the policy says about Google's identifiers and diagnostics", () => {
  const form = repoFile("mobile/store/android/play-forms.md");
  for (const re of [/Firebase installation ID/, /device identifier and a per-installation identifier/, /performance figures, event types and error codes/, /site\/public\/privacy\.html/]) {
    assert.match(form, re, `play-forms.md carries ${re}`);
  }
});
