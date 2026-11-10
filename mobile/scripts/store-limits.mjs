// What Apple allows in a store page, and the names no field of Hide's page may carry: one list, read by the text check
// (check-store-text.mjs) and by the screenshot script (store-shots.mjs), so a headline and a keyword are held to the same rules.

/** Characters Apple accepts per field (App Store Connect, "Platform version information"). */
export const LIMITS = { name: 30, subtitle: 30, promotional_text: 170, keywords: 100, description: 4000, release_notes: 4000 };

/** The review notes field of App Review information. */
export const REVIEW_NOTES_LIMIT = 4000;

/** Other companies' products: never in the name, the subtitle, the keywords or a screenshot's words (guidelines 2.3.7, 4.1(c), 5.2). */
export const THIRD_PARTY_NAMES = [
  'claude', 'anthropic', 'codex', 'chatgpt', 'gpt', 'openai', 'gemini', 'copilot', 'cursor', 'opencode', 'grok', 'antigravity',
  'github', 'tailscale', 'iphone', 'ipad', 'apple', 'ios',
  '클로드', '앤트로픽', '코덱스', '챗gpt', '오픈ai', '제미나이', '깃허브',
];

const ascii = /^[a-z0-9]+$/;

/** The listed names a text contains: an ASCII name as a whole word, a Hangul one anywhere in it. */
export function thirdPartyNamesIn(text) {
  const lower = text.toLowerCase();
  const words = lower.split(/[^a-z0-9]+/).filter(Boolean);
  return THIRD_PARTY_NAMES.filter((name) => (ascii.test(name) ? words.includes(name) : lower.includes(name)));
}

/** A field's length as Apple counts it: characters, not UTF-16 units. */
export const lengthOf = (text) => Array.from(text).length;
