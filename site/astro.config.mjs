import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import { fileURLToPath } from "node:url";
import { EMBED_SCRIPT } from "./src/embed-script.mjs";

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
