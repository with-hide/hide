// What Apple allows in a store page, and the names no field of Hide's page may carry: one list, read by the text check
// (check-store-text.mjs) and by the screenshot script (store-shots.mjs), so a headline and a keyword are held to the same rules.

/** Characters Apple accepts per field (App Store Connect, "Platform version information"). */
export const LIMITS = { name: 30, subtitle: 30, promotional_text: 170, keywords: 100, description: 4000, release_notes: 4000 };

/** Characters Google Play accepts per field (Play Console, "Main store listing" and "Release notes"). */
export const PLAY_LIMITS = { title: 30, short_description: 80, full_description: 4000, changelog: 500 };

/** The review notes field of App Review information. */
export const REVIEW_NOTES_LIMIT = 4000;

/** The coding agents Hide works with, by the names their makers use: a description may name them only with the independence line. */
export const AGENT_NAMES = ['claude code', 'claude', 'codex', 'opencode', 'grok', 'antigravity', 'chatgpt'];

/** Other companies' products: never in the name, the subtitle, the keywords or a screenshot's words (guidelines 2.3.7, 4.1(c), 5.2). */
export const THIRD_PARTY_NAMES = [
  ...AGENT_NAMES.filter((name) => !name.includes(' ')), 'anthropic', 'gpt', 'openai', 'gemini', 'copilot', 'cursor',
  'github', 'tailscale', 'windsurf', 'iphone', 'ipad', 'apple', 'ios',
  '클로드', '앤트로픽', '앤스로픽', '코덱스', '챗gpt', '오픈ai', '제미나이', '깃허브', '커서', '윈드서프',
];

const ascii = /^[a-z0-9]+$/;

/**
 * The listed names a text contains. An ASCII name counts as a whole word, with a camelCase word split (`ClaudeCode`) and
 * two neighbouring words joined (`Open AI`, `git-hub`), so spacing and case do not hide it while `cursory` and `codexes`
 * stay what they are; a Hangul name counts anywhere in the text once its separators are removed (`클로 드`).
 */
export function thirdPartyNamesIn(text) {
  const words = text.replace(/([a-z0-9])([A-Z])/g, '$1 $2').toLowerCase().split(/[^a-z0-9]+/).filter(Boolean);
  const candidates = new Set([...words, ...words.slice(1).map((word, index) => words[index] + word)]);
  const compact = text.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, '');
  return THIRD_PARTY_NAMES.filter((name) => (ascii.test(name) ? candidates.has(name) : compact.includes(name)));
}

/** A field's length as Apple counts it: characters, not UTF-16 units. */
export const lengthOf = (text) => Array.from(text).length;
