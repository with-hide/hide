// Writes `mobile/retained-bundles.json`: the older phone protocols the app carries, each file's size and SHA-256.
//
// This is the one place the production phone site is believed, and it only proposes: the result is a diff a person
// reads in a pull request of its own before a build trusts it (docs/BUILD.md, The iOS app). Every file is downloaded
// and hashed here, and must also agree with the site's inventory, so a half-deployed site fails now rather than
// being pinned. The protocol this source builds itself (`PHONE_PROTOCOL`) is never pinned.
import { readFile, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { ORIGIN, inventory, sha256 } from '../../scripts/phone-release.mjs';
import { PINS, json, parsePins, read } from './assemble.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..');

export async function pin({ current, origin = ORIGIN, fetcher = fetch }) {
  const site = await json(`${origin}/phone-site.json`, fetcher);
  const published = inventory(site);
  if (JSON.stringify(await json(`${origin}/protocols.json`, fetcher)) !== JSON.stringify(published)) {
    throw new Error('The phone site disagrees with itself about its protocols');
  }
  const protocols = {};
  for (const number of published.filter((candidate) => candidate !== current)) {
    const files = {};
    for (const [path, entry] of Object.entries(site.files).filter(([candidate]) => candidate.startsWith(`p/${number}/`)).sort(([a], [b]) => a.localeCompare(b))) {
      const { bytes } = await read(`${origin}/${path}`, fetcher);
      const digest = sha256(bytes);
      if (bytes.length !== entry.size || digest !== entry.sha256) throw new Error(`The phone site's file differs from its own inventory: ${path}`);
      files[path] = { sha256: digest, size: bytes.length };
    }
    protocols[number] = files;
  }
  const value = { schema: 1, protocols };
  parsePins(value);
  return value;
}

/** One line per file, so a pull request's diff reads one pinned file at a time. */
export function text(value) {
  const protocols = Object.entries(value.protocols).map(([number, files]) =>
    `    "${number}": {\n${Object.entries(files).map(([path, entry]) => `      ${JSON.stringify(path)}: ${JSON.stringify(entry)}`).join(',\n')}\n    }`);
  return `{\n  "schema": 1,\n  "protocols": {\n${protocols.join(',\n')}\n  }\n}`;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const source = await readFile(resolve(ROOT, 'web/src/mobile/phoneVersion.ts'), 'utf8');
    const current = Number(source.match(/PHONE_PROTOCOL = (\d+)/)?.[1]);
    if (!Number.isSafeInteger(current) || current < 1) throw new Error('phone protocol missing');
    const value = await pin({ current });
    await writeFile(PINS, `${text(value)}\n`);
    console.log(`pinned protocols ${Object.keys(value.protocols).join(', ')} in mobile/retained-bundles.json; review the diff`);
  } catch (error) {
    console.error(`Pinning stopped: ${error.message}`);
    process.exitCode = 1;
  }
}
