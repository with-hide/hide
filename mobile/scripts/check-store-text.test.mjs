import { cp, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { check } from './check-store-text.mjs';

const real = resolve(dirname(fileURLToPath(import.meta.url)), '..');
let copy;
beforeEach(async () => {
  copy = await mkdtemp(join(tmpdir(), 'store-text-'));
  await cp(join(real, 'store'), join(copy, 'store'), { recursive: true });
});
afterEach(async () => { await rm(copy, { recursive: true, force: true }); });

const path = (file) => join(copy, 'store/metadata', file);
const set = (file, text) => writeFile(path(file), `${text}\n`);
async function edit(file, from, to) {
  const text = await readFile(path(file), 'utf8');
  expect(text).toContain(from);
  await writeFile(path(file), text.replace(from, to));
}

describe('the store text as committed', () => {
  it('passes every limit and rule', async () => {
    expect(await check(real)).toEqual([]);
  });
});

describe("Apple's limits", () => {
  it.each([
    ['name.txt', 31, 'name'],
    ['subtitle.txt', 31, 'subtitle'],
    ['promotional_text.txt', 171, 'promotional_text'],
    ['keywords.txt', 101, 'keywords'],
    ['description.txt', 4001, 'description'],
  ])('refuses %s at %i characters', async (file, length, field) => {
    const text = file === 'keywords.txt' ? 'a'.repeat(length) : file === 'description.txt' ? `withhide.dev\n${'a'.repeat(length - 13)}` : 'a'.repeat(length);
    await set(`en-US/${file}`, text);
    expect(await check(copy)).toEqual(expect.arrayContaining([expect.stringContaining(`en-US/${field}.txt is ${length} characters`)]));
  });

  it('counts a Hangul syllable as one character', async () => {
    await set('ko/promotional_text.txt', '가'.repeat(170));
    expect((await check(copy)).filter((line) => line.includes('promotional_text'))).toEqual([]);
    await set('ko/promotional_text.txt', '가'.repeat(171));
    expect(await check(copy)).toEqual([expect.stringContaining('ko/promotional_text.txt is 171 characters')]);
  });

  it('accepts a field exactly at its limit', async () => {
    await set('en-US/promotional_text.txt', 'a'.repeat(170));
    await set('en-US/keywords.txt', ['terminal', 'developer', 'review', 'mission', 'monitor', 'automation', 'notifications', 'assistant'].join(',') + ',a'.repeat(0));
    expect((await check(copy)).filter((line) => line.includes('characters'))).toEqual([]);
  });
});

describe("other companies' marks", () => {
  it.each(['Claude', 'codex', 'ChatGPT', 'OpenAI', 'GitHub'])('refuses %s in the keywords', async (name) => {
    await set('en-US/keywords.txt', `terminal,${name},review`);
    expect(await check(copy)).toEqual([expect.stringContaining('keywords.txt names')]);
  });

  it('refuses a name in the subtitle and in the Korean spelling', async () => {
    await set('en-US/subtitle.txt', 'Claude on your phone');
    await set('ko/keywords.txt', '클로드,터미널');
    const problems = await check(copy);
    expect(problems).toEqual(expect.arrayContaining([expect.stringContaining('en-US/subtitle.txt'), expect.stringContaining('ko/keywords.txt names 클로드')]));
  });

  it.each(['ClaudeCode', 'claude-code', 'Open AI', 'git hub', 'Windsurf'])('sees %s whatever its spacing or case', async (name) => {
    await set('en-US/keywords.txt', `terminal,${name}`);
    expect(await check(copy)).toEqual([expect.stringContaining('keywords.txt names')]);
  });

  it.each(['앤스로픽', '커서', '클로 드'])('sees the Korean spelling %s', async (name) => {
    await set('ko/keywords.txt', `터미널,${name}`);
    expect(await check(copy)).toEqual([expect.stringContaining('ko/keywords.txt names')]);
  });

  it('does not take a longer word for a name', async () => {
    await set('en-US/keywords.txt', 'terminal,cursory,codexes');
    expect(await check(copy)).toEqual([]);
  });

  it('wants the independence line wherever the description names an agent', async () => {
    await edit('en-US/description.txt', 'is not affiliated with, endorsed by or sponsored by', 'is separate from');
    expect(await check(copy)).toEqual([expect.stringContaining('en-US/description.txt names an agent without saying Hide is independent')]);
  });
});

describe('the keywords', () => {
  it.each([
    ['terminal, developer', 'no space around a comma'],
    ['terminal,,developer', 'no empty item'],
    ['terminal,Terminal', 'repeats'],
    ['terminal,agents', 'already in the name'],
    ['terminal,phone', 'already in the name or the subtitle'],
  ])('refuses %s', async (keywords, why) => {
    await set('en-US/keywords.txt', keywords);
    expect(await check(copy)).toEqual([expect.stringContaining(why)]);
  });
});

describe('what was decided', () => {
  it('holds the name, the English subtitle, the URLs and the categories', async () => {
    await set('en-US/name.txt', 'Hide');
    await set('ko/privacy_url.txt', 'https://example.test/privacy');
    await set('en-US/subtitle.txt', 'Another line');
    await set('primary_category.txt', 'UTILITIES');
    expect(await check(copy)).toEqual(expect.arrayContaining([
      expect.stringContaining('en-US/name.txt is "Hide"'),
      expect.stringContaining('ko/privacy_url.txt'),
      expect.stringContaining('en-US/subtitle.txt is "Another line"'),
      expect.stringContaining('primary_category.txt'),
    ]));
  });

  it('wants the first two lines of a description to say it works with Hide from withhide.dev', async () => {
    await edit('ko/description.txt', 'withhide.dev에서 받는', '웹에서 받는');
    expect(await check(copy)).toEqual([expect.stringContaining('ko/description.txt: the first two lines')]);
  });

  it('refuses a missing file, an em dash and notes over the limit', async () => {
    await rm(path('ko/release_notes.txt'));
    await set('en-US/release_notes.txt', 'First release — hello');
    await set('review_information/notes.txt', `hello@withhide.dev ${'a'.repeat(4000)}`);
    expect(await check(copy)).toEqual(expect.arrayContaining([
      expect.stringContaining('ko/release_notes.txt is missing'),
      expect.stringContaining('en-US/release_notes.txt has an em dash'),
      expect.stringContaining('review_information/notes.txt is'),
    ]));
  });
});

describe("the screenshots' words", () => {
  it('are held to the same naming rule', async () => {
    const file = join(copy, 'store/shots.json');
    const shots = JSON.parse(await readFile(file, 'utf8'));
    shots.shots[0].text.en.support = 'Works with Claude Code.';
    await writeFile(file, JSON.stringify(shots));
    expect(await check(copy)).toEqual([expect.stringContaining('agents: en support names claude')]);
  });
});
