#!/usr/bin/env node
// Holds the guides and the feature pages to each other: a guide names the features it uses in its
// `features:` header, and each feature page's "Used in guides" list is read back from those headers
// (`--write` emits it as JSON for the page's component). It fails on a guide that names no feature page,
// and on a feature page no guide uses.
// Usage: check-features.mjs <content-dir> [--write <used-in.json>]
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import { pages } from "./lib.mjs";

const [root, flag, out] = process.argv.slice(2);
if (!root) throw new Error("usage: check-features.mjs <content-dir> [--write <used-in.json>]");
const guides = pages(root, "guides");
const features = pages(root, "features");
const featureIds = new Set(features.map((feature) => feature.id));
const problems = [];
const usedIn = Object.fromEntries(features.map((feature) => [feature.id, []]));
for (const guide of guides) {
  if (!guide.features?.length) problems.push(`guide ${guide.id}: no \`features:\` header`);
  for (const id of guide.features ?? []) {
    if (!featureIds.has(id)) problems.push(`guide ${guide.id}: names feature "${id}", which has no page`);
    else usedIn[id].push({ id: guide.id, title: guide.title });
  }
}
for (const feature of features) if (usedIn[feature.id].length === 0) problems.push(`feature ${feature.id}: no guide uses it`);
if (problems.length) {
  console.error(problems.map((line) => `  ${line}`).join("\n"));
  process.exit(1);
}
if (flag === "--write") {
  mkdirSync(dirname(out), { recursive: true });
  writeFileSync(out, JSON.stringify(usedIn, null, 2) + "\n");
}
console.log(`ok: ${guides.length} guides, ${features.length} feature pages, every link resolves both ways`);
