// The store page's text is source (mobile/store/metadata, the layout fastlane's `deliver` reads), and Apple's limits and
// Hide's own rules are checked here without App Store Connect, so every pull request can run them:
//
//  - a field is within Apple's length limit (name 30, subtitle 30, promotional text 170, keywords 100, description 4000);
//  - no other company's product name is in the name, the subtitle or the keywords, and where the description names an
//    agent it also says Hide is independent of it;
//  - the keywords are comma separated with no spaces around the commas, repeat nothing, and repeat no word of the name
//    or the subtitle, which Apple already indexes;
//  - the decided facts (name, English subtitle, URLs, categories) are as decided, and the first two lines of a
//    description say it works with Hide on the computer;
//  - Google Play's page is source too (mobile/store/android/metadata, the layout fastlane's `supply` reads) and is held
//    to Play's limits (title 30, short description 80, full description 4000, release notes 500) and the same rules,
//    plus Play's metadata policy where a file can show it (no emoji, no word in capitals);
//  - the screenshots' words in mobile/store/shots.json follow the same naming rule (store-shots.mjs), and the Play list
//    (mobile/store/android/shots.json) names shots that exist, at Play's sizes.
import { readFile, readdir } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { AGENT_NAMES, LIMITS, PLAY_LIMITS, REVIEW_NOTES_LIMIT, lengthOf, thirdPartyNamesIn } from './store-limits.mjs';
import { androidShotProblems, loadAndroidShots, loadShots, shotProblems } from './store-shots.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');

const FIELDS = ['name', 'subtitle', 'description', 'keywords', 'promotional_text', 'release_notes', 'support_url', 'marketing_url', 'privacy_url'];
const LOCALES = ['en-US', 'ko'];
const DECIDED = {
  name: 'Hide - Remote Coding Agents',
  support_url: 'https://withhide.dev/support',
  marketing_url: 'https://withhide.dev',
  privacy_url: 'https://withhide.dev/privacy',
};
const ENGLISH_SUBTITLE = 'Your agent IDE, on your phone';
const CATEGORIES = { primary_category: 'DEVELOPER_TOOLS', secondary_category: 'PRODUCTIVITY' };
// Where a description names an agent it also says Hide is independent of its maker, in the language it is written in.
const INDEPENDENT = { 'en-US': /not affiliated with/i, ko: /제휴/ };
const WORKS_WITH_HIDE = { 'en-US': /withhide\.dev/, ko: /withhide\.dev/ };

const words = (text) => text.toLowerCase().split(/[^\p{L}\p{N}]+/u).filter(Boolean);

async function read(path) {
  try { return (await readFile(path, 'utf8')).replace(/\n$/, ''); } catch { return null; }
}

const PLAY_LOCALES = ['en-US', 'ko-KR'];
// Play's files, by the name fastlane supply reads, and the limit each is held to.
const PLAY_FILES = { title: 'title', short_description: 'short_description', full_description: 'full_description', changelog: 'changelogs/default' };
// The language a Play locale's rules are written in.
const PLAY_LANGUAGE = { 'en-US': 'en-US', 'ko-KR': 'ko' };

/** What is wrong with Google Play's page text, empty when nothing is. */
async function checkPlay(root) {
  const problems = [];
  const base = join(root, 'store/android/metadata');
  const present = (await readdir(base, { withFileTypes: true }).catch(() => [])).filter((entry) => entry.isDirectory()).map((entry) => entry.name).sort();
  if (present.join() !== [...PLAY_LOCALES].sort().join()) problems.push(`mobile/store/android/metadata holds ${present.join(', ') || 'nothing'}, expected ${PLAY_LOCALES.join(', ')}`);
  for (const locale of PLAY_LOCALES) {
    const text = {};
    for (const [field, file] of Object.entries(PLAY_FILES)) {
      const where = `android/${locale}/${file}.txt`;
      const value = await read(join(base, locale, `${file}.txt`));
      if (value === null || value.trim() === '') { problems.push(`${where} is missing or empty`); continue; }
      if (value !== value.trim()) problems.push(`${where} has blank space around its text`);
      if (value.includes('—')) problems.push(`${where} has an em dash`);
      if (/\p{Extended_Pictographic}/u.test(value)) problems.push(`${where} has an emoji, which Play's metadata policy refuses`);
      const shouting = value.match(/\b[A-Z]{4,}\b/g);
      if (shouting) problems.push(`${where} has ${shouting[0]} in capitals, which Play's metadata policy refuses`);
      if (lengthOf(value) > PLAY_LIMITS[field]) problems.push(`${where} is ${lengthOf(value)} characters, over Play's ${PLAY_LIMITS[field]}`);
      text[field] = value;
    }
    if (text.title !== undefined && text.title !== DECIDED.name) problems.push(`android/${locale}/title.txt is "${text.title}", decided "${DECIDED.name}"`);
    for (const field of ['title', 'short_description']) {
      const names = text[field] === undefined ? [] : thirdPartyNamesIn(text[field]);
      if (names.length) problems.push(`android/${locale}/${field}.txt names ${names.join(', ')}, which is another company's mark`);
    }
    if (text.full_description !== undefined) {
      const language = PLAY_LANGUAGE[locale];
      if (!text.full_description.split('\n').slice(0, 2).some((line) => WORKS_WITH_HIDE[language].test(line))) problems.push(`android/${locale}/full_description.txt: the first two lines must say the app works with Hide from withhide.dev`);
      const lower = text.full_description.toLowerCase();
      if (AGENT_NAMES.some((name) => lower.includes(name)) && !INDEPENDENT[language].test(text.full_description)) problems.push(`android/${locale}/full_description.txt names an agent without saying Hide is independent of it`);
    }
  }
  return problems;
}

/** @returns {Promise<string[]>} what is wrong, empty when nothing is */
export async function check(root = ROOT) {
  const problems = [];
  const base = join(root, 'store/metadata');
  for (const [file, expected] of Object.entries(CATEGORIES)) {
    const value = await read(join(base, `${file}.txt`));
    if (value !== expected) problems.push(`${file}.txt is ${JSON.stringify(value)}, expected ${expected}`);
  }
  const copyright = await read(join(base, 'copyright.txt'));
  if (!copyright || !/^\d{4} \S/.test(copyright)) problems.push('copyright.txt is missing or is not "<year> <holder>"');

  for (const locale of LOCALES) {
    const text = {};
    for (const field of FIELDS) {
      const value = await read(join(base, locale, `${field}.txt`));
      if (value === null || value.trim() === '') { problems.push(`${locale}/${field}.txt is missing or empty`); continue; }
      if (value !== value.trim()) problems.push(`${locale}/${field}.txt has blank space around its text`);
      if (value.includes('—')) problems.push(`${locale}/${field}.txt has an em dash`);
      text[field] = value;
    }
    for (const [field, limit] of Object.entries(LIMITS)) {
      if (text[field] !== undefined && lengthOf(text[field]) > limit) problems.push(`${locale}/${field}.txt is ${lengthOf(text[field])} characters, over Apple's ${limit}`);
    }
    for (const [field, value] of Object.entries(DECIDED)) {
      if (text[field] !== undefined && text[field] !== value) problems.push(`${locale}/${field}.txt is "${text[field]}", decided "${value}"`);
    }
    if (locale === 'en-US' && text.subtitle !== undefined && text.subtitle !== ENGLISH_SUBTITLE) problems.push(`en-US/subtitle.txt is "${text.subtitle}", decided "${ENGLISH_SUBTITLE}"`);

    for (const field of ['name', 'subtitle', 'keywords']) {
      const names = text[field] === undefined ? [] : thirdPartyNamesIn(text[field]);
      if (names.length) problems.push(`${locale}/${field}.txt names ${names.join(', ')}, which is another company's or Apple's mark`);
    }

    if (text.keywords !== undefined) {
      const list = text.keywords.split(',');
      if (list.some((word) => word === '' || word !== word.trim())) problems.push(`${locale}/keywords.txt must be words separated by commas with no empty item and no space around a comma`);
      const seen = new Set();
      for (const word of list) {
        if (seen.has(word.toLowerCase())) problems.push(`${locale}/keywords.txt repeats "${word}"`);
        seen.add(word.toLowerCase());
      }
      const indexed = new Set([...words(text.name ?? ''), ...words(text.subtitle ?? '')]);
      for (const word of list.flatMap((item) => words(item))) {
        if (indexed.has(word)) problems.push(`${locale}/keywords.txt has "${word}", already in the name or the subtitle`);
      }
    }

    if (text.description !== undefined) {
      const lines = text.description.split('\n');
      if (!lines.slice(0, 2).some((line) => WORKS_WITH_HIDE[locale].test(line))) problems.push(`${locale}/description.txt: the first two lines must say the app works with Hide from withhide.dev`);
      const lower = text.description.toLowerCase();
      if (AGENT_NAMES.some((name) => lower.includes(name)) && !INDEPENDENT[locale].test(text.description)) problems.push(`${locale}/description.txt names an agent without saying Hide is independent of it`);
    }
  }

  const notes = await read(join(base, 'review_information/notes.txt'));
  if (notes === null) problems.push('review_information/notes.txt is missing');
  else {
    if (lengthOf(notes) > REVIEW_NOTES_LIMIT) problems.push(`review_information/notes.txt is ${lengthOf(notes)} characters, over ${REVIEW_NOTES_LIMIT}`);
    if (!notes.includes('hello@withhide.dev')) problems.push('review_information/notes.txt names no contact');
    if (notes.includes('—')) problems.push('review_information/notes.txt has an em dash');
  }
  const locales = (await readdir(base, { withFileTypes: true })).filter((entry) => entry.isDirectory()).map((entry) => entry.name).sort();
  const expected = [...LOCALES, 'review_information'].sort();
  if (locales.join() !== expected.join()) problems.push(`mobile/store/metadata holds ${locales.join(', ')}, expected ${expected.join(', ')}`);

  const shots = await loadShots(root);
  return [...problems, ...(await checkPlay(root)), ...shotProblems(shots), ...androidShotProblems(await loadAndroidShots(root), shots)];
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  const problems = await check();
  if (problems.length) {
    process.stderr.write(`store text check failed:\n${problems.map((line) => `  ${line}`).join('\n')}\n`);
    process.exit(1);
  }
  process.stdout.write('store text ok\n');
}
