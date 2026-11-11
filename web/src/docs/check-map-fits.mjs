// Proves the mission-map part is shown whole in a docs page's frame, at the two widths the pages give it (1440 and 390
// pixel viewports) in both themes: a frame that clips the map's summary leaves hidden content in the part's own
// scroller, which `scrollHeight <= clientHeight` reads. The frame sits in a stand-in page that runs the real
// EMBED_SCRIPT (site/src/embed-script.mjs), so the height the part posts is applied to the iframe by the code the
// site ships, and the check also fails when that height never settles (a growth loop between part and page) or leaves blank space under the map (a frame grown past what the map needs).
//
//   pnpm --dir web build:docs-embed && node web/src/docs/check-map-fits.mjs
//
// Needs Playwright's Chromium like check-still.mjs. There is no timed wait: "settled" means the same picture for
// SETTLED_FRAMES animation frames in a row, and a page that has not settled after MAX_FRAMES has grown without end.
/* global console, URL, document, window, requestAnimationFrame */
import { existsSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { launchChromium, serveDirectory } from "../../../scripts/docs/browser.mjs";
import { EMBED_SCRIPT } from "../../../site/src/embed-script.mjs";

const web = fileURLToPath(new URL("../../", import.meta.url));
const dist = join(web, "docs-embed-dist");
if (!existsSync(join(dist, "docs-embed.html"))) throw new Error(`${dist} is missing: run \`pnpm --dir web build:docs-embed\` first`);

// The width a docs page gives a frame at each viewport (measured on /docs/guides/run-a-mission/), and the height its page writes.
const CASES = [{ viewport: 1440, frame: 736 }, { viewport: 390, frame: 358 }];
const FRAME_ATTRIBUTE_HEIGHT = 825;
const SETTLED_FRAMES = 30;
// The frame grows by what is hidden and no more: the box may end this many pixels below the last line of the map.
const BLANK_TOLERANCE = 2;
const MAX_FRAMES = 1200;

const host = (theme, width) => `<!doctype html><html data-theme="${theme}"><head><meta charset="utf-8"><style>html,body{margin:0}iframe{display:block;border:0;width:${width}px}</style></head><body>
<script>window.__heights = []; window.addEventListener("message", function (event) { if (event.data && event.data.type === "hide-docs-embed-height") window.__heights.push(event.data.height); });</script>
<iframe src="/embed/docs-embed.html?part=mission-map&theme=${theme}&lang=en" height="${FRAME_ATTRIBUTE_HEIGHT}"></iframe>
<script>${EMBED_SCRIPT}</script></body></html>`;

const pages = {};
for (const theme of ["dark", "light"]) for (const { frame } of CASES) pages[`/host-${theme}-${frame}.html`] = host(theme, frame);
const { base, close } = await serveDirectory(dist, { mounts: { "/embed": dist }, pages });
const browser = await launchChromium("check-map-fits");

/** Runs in the page: waits for the picture to stop changing and reports what is hidden. */
async function settle({ settledFrames, maxFrames }) {
  const frame = document.querySelector("iframe");
  const read = () => {
    const inner = frame.contentDocument;
    const scroller = inner && inner.querySelector("[data-mission-scroll]");
    if (!scroller) return null;
    return {
      posts: window.__heights.length,
      lastPost: window.__heights[window.__heights.length - 1] ?? null,
      frameHeight: frame.style.height,
      scroll: scroller.scrollHeight,
      content: Math.ceil(Math.max(...[...scroller.children].map((child) => child.getBoundingClientRect().bottom)) - scroller.getBoundingClientRect().top),
      client: scroller.clientHeight,
      documentScroll: inner.documentElement.scrollHeight,
      documentClient: inner.documentElement.clientHeight,
    };
  };
  let previous = null;
  let same = 0;
  for (let frames = 0; frames < maxFrames; frames++) {
    await new Promise((next) => requestAnimationFrame(next));
    const now = read();
    same = now && previous && JSON.stringify(now) === JSON.stringify(previous) && now.posts > 0 ? same + 1 : 0;
    previous = now;
    if (same >= settledFrames) return { settled: true, ...now, frameRendered: Math.round(frame.getBoundingClientRect().height) };
  }
  return { settled: false, ...previous };
}

const failures = [];
try {
  for (const theme of ["dark", "light"]) {
    for (const { viewport, frame } of CASES) {
      const context = await browser.newContext({ viewport: { width: viewport, height: 900 }, reducedMotion: "reduce" });
      const page = await context.newPage();
      const errors = [];
      page.on("pageerror", (error) => errors.push(error.message));
      await page.goto(`${base}/host-${theme}-${frame}.html`);
      const got = await page.evaluate(settle, { settledFrames: SETTLED_FRAMES, maxFrames: MAX_FRAMES });
      await context.close();
      const problems = [];
      if (!got.settled) problems.push(`the height did not settle in ${MAX_FRAMES} frames (posts ${got.posts}, last ${got.lastPost})`);
      else {
        if (got.scroll > got.client) problems.push(`${got.scroll - got.client} px of the map are hidden (scrollHeight ${got.scroll} > clientHeight ${got.client})`);
        if (got.client - got.content > BLANK_TOLERANCE) problems.push(`${got.client - got.content} px of blank space under the map (content ends at ${got.content}, the box is ${got.client})`);
        if (got.documentScroll > got.documentClient) problems.push(`the part's page scrolls (${got.documentScroll} > ${got.documentClient})`);
        if (got.frameRendered !== got.lastPost) problems.push(`the frame is ${got.frameRendered} px but the part's last post is ${got.lastPost}`);
        if (got.lastPost >= 2000) problems.push(`the posted height ${got.lastPost} reached the page's clamp`);
      }
      problems.push(...errors);
      console.log(`${problems.length === 0 ? "ok  " : "FAIL"} mission-map ${theme.padEnd(5)} ${String(viewport).padStart(4)} px: frame ${frame} wide, ${got.lastPost} tall after ${got.posts} posts (scroller ${got.scroll}/${got.client})${problems.length ? ` - ${problems.join("; ")}` : ""}`);
      if (problems.length) failures.push(`${theme} ${viewport}`);
    }
  }
} finally {
  await browser.close();
  await close();
}
if (failures.length > 0) throw new Error(`The mission map is not shown whole: ${failures.join("; ")}`);
console.log("the mission map is shown whole and its height settles, at 1440 and 390, dark and light");
