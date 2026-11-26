import { describe, expect, it } from "vitest";
import { markdownBlockHolding, markdownPlainText } from "./markdownPlain";

describe("markdownPlainText", () => {
  it("keeps the words and drops the marks and the blank lines", () => {
    expect(markdownPlainText("## 배경\n\n출처를 **어댑터**로 나눈다.\n\n- GitHub\n- [x] `Local`\n> 인용 [링크](https://example.invalid)")).toBe(
      "배경\n출처를 어댑터로 나눈다.\nGitHub\nLocal\n인용 링크",
    );
  });
});

describe("markdownBlockHolding", () => {
  const answer = "PR을 올렸습니다.\n\n남은 건 두 가지예요.\n- **배포:**  지금 `main`에\n  배포할까요?\n- 127번 칩은 하나로 줄일지 알려 주세요.\n\n다크 화면도 넣을까요? 넣으면 캡처가\n두 배가 돼요.\n\n> 인용 안의 \"질문\"인가요?";

  it("gives the list item or the paragraph that holds the sentence, read as the quote check reads it", () => {
    expect(markdownBlockHolding(answer, "배포: 지금 main에 배포할까요?")).toBe("- **배포:**  지금 `main`에\n  배포할까요?");
    expect(markdownBlockHolding(answer, "다크 화면도 넣을까요?")).toBe("다크 화면도 넣을까요? 넣으면 캡처가\n두 배가 돼요.");
    expect(markdownBlockHolding(answer, "인용 안의 질문인가요?")).toBe("> 인용 안의 \"질문\"인가요?");
  });

  it("gives none when the sentence is not there, is empty or runs across blocks", () => {
    expect(markdownBlockHolding(answer, "앞 답변에서 물은 질문인가요?")).toBeNull();
    expect(markdownBlockHolding(answer, " ** ")).toBeNull();
    expect(markdownBlockHolding(answer, "올렸습니다. 남은 건")).toBeNull();
  });
});
