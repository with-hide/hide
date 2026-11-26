import { describe, expect, it } from "vitest";
import { markdownPlainText, sentenceIn } from "./markdownPlain";

describe("markdownPlainText", () => {
  it("keeps the words and drops the marks and the blank lines", () => {
    expect(markdownPlainText("## 배경\n\n출처를 **어댑터**로 나눈다.\n\n- GitHub\n- [x] `Local`\n> 인용 [링크](https://example.invalid)")).toBe(
      "배경\n출처를 어댑터로 나눈다.\nGitHub\nLocal\n인용 링크",
    );
  });
});

describe("sentenceIn", () => {
  const answer = "PR을 올렸습니다.\n\n- **배포:**  지금 `main`에\n  배포할까요?\n\n> 인용 안의 \"질문\"인가요?";

  it("finds the sentence however the text marks it up, from its first character to its last", () => {
    const at = (quote: string) => {
      const found = sentenceIn(answer, quote);
      return found && answer.slice(found.from, found.to);
    };
    expect(at("배포: 지금 main에 배포할까요?")).toBe("배포:**  지금 `main`에\n  배포할까요?");
    expect(at("인용 안의 질문인가요?")).toBe("인용 안의 \"질문\"인가요?");
  });

  it("gives none when the sentence is not there or is empty", () => {
    expect(sentenceIn(answer, "앞 답변에서 물은 질문인가요?")).toBeNull();
    expect(sentenceIn(answer, " ** ")).toBeNull();
  });
});
