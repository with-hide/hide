import { defineCollection, z } from "astro:content";
import { docsLoader } from "@astrojs/starlight/loaders";
import { docsSchema } from "@astrojs/starlight/schema";

// A guide names the features it uses in `features: [id, ...]`; `scripts/docs/check-features.mjs` holds
// that list to the feature pages and writes each feature's "Used in guides" list from it.
export const collections = {
  docs: defineCollection({ loader: docsLoader(), schema: docsSchema({ extend: z.object({ features: z.array(z.string()).optional() }) }) }),
};
