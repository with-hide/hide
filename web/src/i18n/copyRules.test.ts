import { describe, expect, it } from "vitest";
import { catalogs } from "./catalogs";

// The mechanical half of "Writing the copy" in docs/LOCALIZATION.md: the rules a
// scan can decide. Naturalness, term choice beyond the banned words and ja/zh
// wording still need a reader.
const ko = catalogs.ko as Record<string, string>;
const zh = catalogs["zh-CN"] as Record<string, string>;
const ja = catalogs.ja as Record<string, string>;

// Prompts typed into an agent are not screen copy (docs/LOCALIZATION.md, Writing the copy).
const AGENT_PROMPTS = new Set(["issue.prompt.local", "issue.prompt.github", "issue.prompt.closes"]);

function violations(table: Record<string, string>, pattern: RegExp): string[] {
  return Object.keys(table)
    .filter((key) => !AGENT_PROMPTS.has(key) && pattern.test(table[key]!))
    .map((key) => `${key}: ${table[key]!.slice(0, 80)}`);
}

describe("the copy rules, as far as a scan can decide them", () => {
  it("puts no particle that depends on the last sound right after an inserted value", () => {
    expect(violations(ko, /\}\}(은|는|이|가|을|를|과|와|으로|로)(?![가-힣])/)).toEqual([]);
  });

  it("writes no 을(를)-style pair", () => {
    expect(violations(ko, /[가-힣]\((을|를|이|가|은|는|와|과|으로|로)\)|\((을|를|이|가|으)\)[가-힣]/)).toEqual([]);
  });

  it("speaks Korean in 해요체, never 합니다체 or 반말", () => {
    expect(violations(ko, /(합니다|입니다|습니다|됩니다|십시오)(?![가-힣])/)).toEqual([]);
    expect(violations(ko, /(했다|된다|한다|이다|없다|있다|않는다)[.]?$/)).toEqual([]);
  });

  it("uses the glossary's word, not a word it retired", () => {
    expect(violations(ko, /일감|일하는 중|풀 리퀘스트|하위 에이전트|해지|잇기|잇습니다|디바이스|워크스페이스|페인|레일/)).toEqual([]);
    expect(violations(zh, /代理|您|窗格栏|拉取请求/)).toEqual([]);
    expect(violations(ja, /プルリクエスト|ペーン|課題/)).toEqual([]);
  });

  it("leaves no bare English glossary word in a Korean sentence", () => {
    expect(violations(ko, /(?<![A-Za-z`{/._-])\b(checkout|workspace|worktree|session|device|merge|clone|scan|loading|retry)\b(?![A-Za-z`}/._-])/)).toEqual([]);
  });
});
