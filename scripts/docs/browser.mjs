// What the docs checks that drive a browser share: a loopback static server for a built folder and Playwright's
// Chromium from the web package, started with the cure named when it is missing.
/* global URL */
import { existsSync, readFileSync, statSync } from "node:fs";
import { createServer } from "node:http";
import { createRequire } from "node:module";
import { extname, join, normalize, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(fileURLToPath(new URL("../..", import.meta.url)));
const TYPES = { ".html": "text/html; charset=utf-8", ".js": "text/javascript", ".mjs": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".png": "image/png", ".webp": "image/webp", ".json": "application/json", ".wasm": "application/wasm", ".woff2": "font/woff2", ".xml": "application/xml", ".txt": "text/plain" };

/**
 * Serves `dir` on a free loopback port. `cleanUrls` answers `/a/b/` and `/a/b` from `a/b/index.html` or `a/b.html`, as the
 * site's host does. `mounts` maps a URL prefix to another folder (`{ "/embed": partsDir }`); `pages` maps a path to an
 * html string a check made up. Returns `{ base, close }`.
 */
export async function serveDirectory(dir, { cleanUrls = false, mounts = {}, pages = {} } = {}) {
  const roots = [...Object.entries(mounts).map(([prefix, folder]) => [prefix, resolve(folder)]), ["", resolve(dir)]];
  const server = createServer((request, response) => {
    const path = normalize(decodeURIComponent(new URL(request.url, "http://x").pathname)).replace(/^(\.\.[/\\])+/, "");
    if (path in pages) return response.writeHead(200, { "content-type": TYPES[".html"] }).end(pages[path]);
    for (const [prefix, folder] of roots) {
      if (prefix && path !== prefix && !path.startsWith(`${prefix}/`)) continue;
      const rest = path.slice(prefix.length) || "/";
      const candidates = cleanUrls ? (rest.endsWith("/") ? [join(rest, "index.html")] : [rest, `${rest}.html`, join(rest, "index.html")]) : [rest === "/" ? "/docs-embed.html" : rest];
      for (const candidate of candidates) {
        const file = join(folder, candidate);
        if (file.startsWith(folder) && existsSync(file) && statSync(file).isFile()) return response.writeHead(200, { "content-type": TYPES[extname(file)] ?? "application/octet-stream" }).end(readFileSync(file));
      }
    }
    return response.writeHead(404).end();
  });
  await new Promise((done) => server.listen(0, "127.0.0.1", done));
  return { base: `http://127.0.0.1:${server.address().port}`, close: () => new Promise((done) => server.close(done)) };
}

/** Playwright's Chromium from the web package; a missing browser is a failure with its cure, never a skipped check. */
export async function launchChromium(who) {
  const { chromium } = createRequire(join(root, "web/package.json"))("@playwright/test");
  return chromium.launch().catch((error) => {
    throw new Error(`${who} needs Playwright's Chromium and could not start it: run \`bash scripts/verify-web.sh web playwright-install\` (${error.message.split("\n")[0]})`);
  });
}
