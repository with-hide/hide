import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import { fileURLToPath } from "node:url";

// A live part is framed with `&theme=dark|light`; this keeps each frame on the page's own theme, now and
// when the reader flips Starlight's theme switch, so a dark picture never sits on a light page.
const EMBED_THEME = `(function () {
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
      head: [{ tag: "script", content: EMBED_THEME }],
      sidebar: [
        { label: "Start", autogenerate: { directory: "docs/start" } },
        { label: "Guides", autogenerate: { directory: "docs/guides" } },
        { label: "Features", autogenerate: { directory: "docs/features" } },
        { label: "Reference", autogenerate: { directory: "docs/reference" } },
      ],
    }),
  ],
});
