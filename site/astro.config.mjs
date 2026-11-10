import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import { fileURLToPath } from "node:url";

// Two small behaviors for the live parts a page frames (`/embed/docs-embed.html?part=<id>&theme=...`):
// - each frame stays on the page's own theme, now and when the reader flips Starlight's theme switch, so a
//   dark picture never sits on a light page;
// - a part tells its page how tall it is with `{ type: "hide-docs-embed-height", part, height }` posted to
//   `window.parent`; the page finds the frame that sent it (`event.source`), accepts only its own origin and
//   sets that frame's height, clamped so a wrong number cannot make a page unusable. A page whose part posts
//   nothing keeps the frame's `height` attribute, with the minimum set in `owl-field.css`.
const EMBED_SCRIPT = `(function () {
  function sync() {
    var theme = document.documentElement.dataset.theme === "light" ? "light" : "dark";
    document.querySelectorAll('iframe[src*="/embed/"]').forEach(function (frame) {
      var url = new URL(frame.getAttribute("src"), location.href);
      if (url.searchParams.get("theme") === theme) return;
      url.searchParams.set("theme", theme);
      frame.setAttribute("src", url.pathname + url.search);
    });
  }
  document.addEventListener("DOMContentLoaded", sync);
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
