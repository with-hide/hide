#!/usr/bin/env node
// Every agent logo the web shell bundles has a manifest entry with its source
// URL and licence note, every shared adapter has a logo or a stated reason
// for a monogram, and no logo file is bundled without a manifest entry
// (design principle 10: never render a logo the system cannot source). A logo
// may also belong to a Hide AI provider's agent (hide-ai/src/registry.rs), which
// the Hide AI tab draws although the kit does not list that agent.
// A retired agent may keep its logo for stored history when the manifest
// explicitly marks it history_only and explains the retained reader.
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const dir = path.join(root, 'web/src/assets/agents');
const LOGO_FORMATS = ['svg', 'png'];
const manifest = JSON.parse(fs.readFileSync(path.join(dir, 'manifest.json'), 'utf8'));
const adapters = JSON.parse(fs.readFileSync(path.join(root, 'contracts/agent-adapters.json'), 'utf8')).map(row => row.logo_id);
if (adapters.length === 0) throw new Error('no adapter logo ids read from contracts/agent-adapters.json');
const providerAgents = [...fs.readFileSync(path.join(root, 'hide-ai/src/registry.rs'), 'utf8').matchAll(/^\s+agent: "([^"]+)",$/gm)].map(match => match[1]);
if (providerAgents.length === 0) throw new Error('no provider agent ids read from hide-ai/src/registry.rs');
const problems = [];
const claimed = new Map();
const claim = (id, how, entry) => {
  const active = adapters.includes(id) || providerAgents.includes(id);
  const historyOnly = entry.history_only === true;
  if ('history_only' in entry && !historyOnly) problems.push(`${id}: history_only is either true or absent`);
  if (historyOnly && active) problems.push(`${id}: an active adapter or provider cannot be history_only`);
  if (historyOnly && !entry.note?.trim()) problems.push(`${id}: history_only needs a note identifying the retained reader`);
  if (!active && !historyOnly) problems.push(`${id} (${how}) is neither an adapter in contracts/agent-adapters.json, a provider agent in hide-ai/src/registry.rs, nor declared history_only`);
  if (claimed.has(id)) problems.push(`${id} is both ${claimed.get(id)} and ${how}`);
  claimed.set(id, how);
};
for (const logo of manifest.logos) {
  claim(logo.id, 'bundled', logo);
  if (!/^https:\/\/\S+$/.test(logo.source_url ?? '')) problems.push(`${logo.id}: no https source_url`);
  if (!logo.license?.trim()) problems.push(`${logo.id}: no license note`);
  if (!logo.modification?.trim()) problems.push(`${logo.id}: no modification note`);
  if (!/^\d{4}-\d{2}-\d{2}$/.test(logo.retrieved ?? '')) problems.push(`${logo.id}: no retrieved date`);
  if (!fs.existsSync(path.join(dir, logo.file))) problems.push(`${logo.id}: ${logo.file} is missing`);
  // web/src/agentLogos.ts seats a logo on the plate (--logo-plate) only when it says so; any other value would silently mean no.
  if ('plate' in logo && logo.plate !== true) problems.push(`${logo.id}: plate is either true or absent`);
  // web/src/agentLogos.ts maps a bundled logo to its agent by file name, and globs only these formats.
  if (!LOGO_FORMATS.some(format => logo.file === `${logo.id}.${format}`)) problems.push(`${logo.id}: ${logo.file} must be named ${LOGO_FORMATS.map(format => `${logo.id}.${format}`).join(' or ')}, the names agentLogos.ts reads the agent from`);
}
for (const entry of manifest.existing) {
  claim(entry.id, 'existing', entry);
  if (!fs.existsSync(path.resolve(dir, entry.file))) problems.push(`${entry.id}: ${entry.file} is missing`);
}
for (const entry of manifest.monogram) {
  claim(entry.id, 'monogram', entry);
  if (!entry.reason?.trim()) problems.push(`${entry.id}: a monogram needs its reason`);
}
for (const id of adapters) if (!claimed.has(id)) problems.push(`${id} has neither a logo nor a monogram entry`);
const listed = new Set(manifest.logos.map(logo => logo.file));
for (const file of fs.readdirSync(dir)) {
  if (file === 'manifest.json') continue;
  if (!listed.has(file)) problems.push(`${file} is bundled without a manifest entry`);
}
if (problems.length) {
  console.error(`Agent logo contract failed:\n- ${problems.join('\n- ')}`);
  process.exit(1);
}
console.log(`agent logos ok: ${manifest.logos.length} bundled, ${manifest.existing.length} existing, ${manifest.monogram.length} monogram, ${adapters.length} adapters`);
