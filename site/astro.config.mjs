import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import { fileURLToPath } from "node:url";

// Two small behaviors for the live parts a page frames (`/embed/docs-embed.html?part=<id>&theme=...`):
// - each frame ends on the page's own theme, now and when the reader flips Starlight's theme switch, so a
//   dark picture never sits on a light page. A frame is set to the page's theme as soon as the parser adds it
//   and again at DOMContentLoaded, which is only a hint: changing `src` while the first navigation is still
//   running can lose the race and leave the frame's document on the old theme with the new `src` printed on
//   it. What the page trusts is the document a frame actually loaded, so every frame `load` (the capture
//   phase sees a load that does not bubble) reads that document's own URL and, when its theme is not the
//   page's, replaces the frame's location with the right one; each load is checked again, so it settles;
// - a part tells its page how tall it is with `{ type: "hide-docs-embed-height", part, height }` posted to
//   `window.parent`; the page finds the frame that sent it (`event.source`), accepts only its own origin and
//   sets that frame's height, clamped so a wrong number cannot make a page unusable. A page whose part posts
//   nothing keeps the frame's `height` attribute, with the minimum set in `owl-field.css`.
const EMBED_SCRIPT = `(function () {
  function pageTheme() {
    var theme = document.documentElement.dataset.theme;
    return theme === "light" || theme === "dark" ? theme : null;
  }
  function withTheme(href, theme) {
    var url = new URL(href, location.href);
    if (url.origin !== location.origin || url.searchParams.get("theme") === theme) return null;
    url.searchParams.set("theme", theme);
    return url;
  }
  function frames() {
    return document.querySelectorAll('iframe[src*="/embed/"]');
  }
  function sync() {
    var theme = pageTheme();
    if (!theme) return;
    frames().forEach(function (frame) {
      var url = withTheme(frame.getAttribute("src"), theme);
      if (url) frame.setAttribute("src", url.pathname + url.search);
    });
  }
  function settle(frame) {
    var theme = pageTheme();
    if (!theme || !(frame instanceof HTMLIFrameElement) || !frame.matches('[src*="/embed/"]')) return;
    var loaded;
    try { loaded = frame.contentWindow.location.href; } catch (error) { return; }
    var url = withTheme(loaded, theme);
    if (url && loaded !== "about:blank") frame.contentWindow.location.replace(url.href);
  }
  document.addEventListener("load", function (event) { settle(event.target); }, true);
  var parsing = new MutationObserver(sync);
  parsing.observe(document.documentElement, { childList: true, subtree: true });
  document.addEventListener("DOMContentLoaded", function () {
    parsing.disconnect();
    sync();
  });
  new MutationObserver(sync).observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
  window.addEventListener("message", function (event) {
    var data = event.data;
    if (event.origin !== location.origin || !data || data.type !== "hide-docs-embed-height") return;
    var height = Number(data.height);
    if (!isFinite(height) || height <= 0) return;
    var frames = document.querySelectorAll("iframe");
    for (var i = 0; i < frames.length; i++) {
      if (frames[i].contentWindow === event.source) {
        frames[i].style.height = Math.min(Math.max(Math.round(height), 40), 2000) + "px";
        return;
      }
    }
  });
})();`;

// One project for withhide.dev: `public/` is the landing page, served at `/` exactly as it is on disk,
// and Starlight owns `/docs`. The content folder `src/content/docs/docs/` carries the `docs/` prefix
// itself, so a page's address is its path under that folder (`docs/guides/x.md` is `/docs/guides/x/`).
export default defineConfig({
  site: "https://withhide.dev",
  vite: { resolve: { alias: { "@docs": fileURLToPath(new URL("./src/components", import.meta.url)) } } },
  integrations: [
    starlight({
      title: "hide docs",
      description: "How to run agents in hide, from the first ten minutes to missions, letters and your phone.",
      favicon: "/assets/hide-icon.svg",
      customCss: ["./src/styles/owl-field.css"],
      components: { SiteTitle: "./src/components/SiteTitle.astro", SocialIcons: "./src/components/SiteLinks.astro" },
      head: [{ tag: "script", content: EMBED_SCRIPT }],
      sidebar: [
        { label: "Start", autogenerate: { directory: "docs/start" } },
        { label: "Guides", autogenerate: { directory: "docs/guides" } },
        { label: "Features", autogenerate: { directory: "docs/features" } },
        { label: "Reference", autogenerate: { directory: "docs/reference" } },
      ],
    }),
  ],
});
