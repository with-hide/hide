// Shared by the user-docs checks (docs-user-docs P0): reads the content folder's pages and their frontmatter.
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

/** Reads `key: value` and `key: [a, b]` lines between the first two `---` lines; nothing else. */
export function frontmatter(path) {
  const text = readFileSync(path, "utf8");
  const match = text.match(/^---\n([\s\S]*?)\n---\n/);
  if (!match) throw new Error(`${path}: no frontmatter`);
  const fields = {};
  for (const line of match[1].split("\n")) {
    const pair = line.match(/^([a-z_]+):\s*(.*)$/);
    if (!pair) throw new Error(`${path}: cannot read frontmatter line "${line}"`);
    const [, key, raw] = pair;
    fields[key] = raw.startsWith("[") ? raw.slice(1, -1).split(",").map((item) => item.trim()).filter(Boolean) : raw.replace(/^"|"$/g, "");
  }
  return fields;
}

/** Every page of one section: `{ id, path, ...frontmatter }`. */
export function pages(root, section) {
  const dir = join(root, section);
  return readdirSync(dir).filter((name) => name.endsWith(".md") || name.endsWith(".mdx")).map((name) => {
    const path = join(dir, name);
    return { id: name.replace(/\.mdx?$/, ""), path, ...frontmatter(path) };
  });
}
