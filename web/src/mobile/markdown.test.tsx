import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { renderMarkdown } from "./markdown";

const html = (text: string) => renderToStaticMarkup(<>{renderMarkdown(text)}</>);
const html2 = (text: string) => renderToStaticMarkup(<>{renderMarkdown(text, { html: true })}</>);

describe("renderMarkdown", () => {
  it("draws the syntax an agent writes and drops its marks", () => {
    const out = html("## 결과\n\n**굵게**, *기울임*, `code`, ~~취소~~\n\n- 하나\n- 둘\n\n1. 첫째\n2. 둘째\n\n> 인용\n\n```ts\nconst a = 1;\n```");
    expect(out).toContain('<p class="whitespace-pre-line font-semibold" data-markdown-heading="2">결과</p>');
    expect(out).toContain('<strong class="font-semibold">굵게</strong>');
    expect(out).toContain("<em>기울임</em>");
    expect(out).toContain('<code class="rounded-xs bg-secondary px-xxs font-mono">code</code>');
    expect(out).toContain("<s>취소</s>");
    expect(out).toMatch(/<ul[^>]*><li[^>]*><p[^>]*>하나<\/p><\/li><li[^>]*><p[^>]*>둘<\/p><\/li><\/ul>/);
    expect(out).toMatch(/<ol start="1"[^>]*>/);
    expect(out).toContain("인용</p></blockquote>");
    expect(out).toMatch(/<pre[^>]*>const a = 1;<\/pre>/);
    expect(out).not.toMatch(/\*\*|##|```|~~/);
  });

  it("keeps a link and an image as text and never a target", () => {
    const out = html("[문서](https://example.com/docs) ![그림](a.png) <https://example.com/auto> https://example.com/bare");
    expect(out).toContain("<span>문서</span>");
    expect(out).toContain("<span>그림</span>");
    expect(out).toContain("https://example.com/auto");
    expect(out).toContain("https://example.com/bare");
    expect(out).not.toContain("example.com/docs");
    expect(out).not.toContain("<a");
    expect(out).not.toContain("<img");
  });

  it("draws a table as rows of cells and escapes raw HTML", () => {
    const out = html("| 이름 | 값 |\n| --- | --- |\n| a | **1** |\n\n<script>alert(1)</script>");
    expect(out).toMatch(/<th[^>]*>이름<\/th><th[^>]*>값<\/th>/);
    expect(out).toMatch(/<td[^>]*>a<\/td><td[^>]*><strong[^>]*>1<\/strong><\/td>/);
    expect(out).not.toContain("---");
    expect(out).toContain("&lt;script&gt;");
  });

  it("draws a pull request body's allowed HTML as elements, folding a details box around the Markdown inside it", () => {
    const body = "<details>\n<summary>Logs <b>here</b></summary>\n\n**inside** it\n\n</details>\n\nline<br>next <kbd>⌘K</kbd> H<sub>2</sub>O x<sup>2</sup> <i>it</i> <code>c</code>\n\n<p>para</p>\n\n<img src=\"https://example.com/a.png?x=1&amp;y=2\" alt=\"shot\" width=\"300\">\n\n<a href=\"https://example.com/docs\">docs</a> [md](https://example.com/md) ![pic](https://example.com/p.png)\n\n<!-- hidden -->";
    const out = html2(body);
    expect(out).toMatch(/<details[^>]*><summary[^>]*>Logs <strong[^>]*>here<\/strong><\/summary><p[^>]*><strong[^>]*>inside<\/strong> it<\/p><\/details>/);
    expect(out).toContain("line<br/>next");
    expect(out).toMatch(/<kbd[^>]*>⌘K<\/kbd>/);
    expect(out).toContain("H<sub>2</sub>O x<sup>2</sup> <em>it</em>");
    expect(out).toMatch(/<code[^>]*>c<\/code>/);
    expect(out).toMatch(/<p[^>]*>para<\/p>/);
    expect(out).toMatch(/<img src="https:\/\/example.com\/a.png\?x=1&amp;y=2" alt="shot" width="300"/);
    expect(out).toMatch(/<a href="https:\/\/example.com\/docs" target="_blank" rel="noopener noreferrer"[^>]*>docs<\/a>/);
    expect(out).toMatch(/<a href="https:\/\/example.com\/md"[^>]*>md<\/a>/);
    expect(out).toMatch(/<img src="https:\/\/example.com\/p.png" alt="pic"/);
    expect(out).not.toContain("hidden");
  });

  it("leaves a script, a frame, a style, an event attribute, a javascript: link and any other tag as text in a pull request body", () => {
    const body = "<script>alert(1)</script>\n\n<iframe src=\"https://example.com\"></iframe>\n\n<style>p{}</style>\n\nx <img src=\"https://example.com/a.png\" onerror=\"alert(1)\"> <a href=\"javascript:alert(1)\">go</a> <b onclick=\"x()\">b</b> <span>s</span> <a href=\"/relative\">r</a> [j](javascript:alert(1))";
    const out = html2(body);
    for (const literal of ["&lt;script&gt;", "&lt;iframe", "&lt;style&gt;", "onerror=", "&lt;a href=&quot;javascript:", "onclick=", "&lt;span&gt;", "&lt;a href=&quot;/relative&quot;&gt;"]) expect(out).toContain(literal);
    expect(out).not.toMatch(/<(script|iframe|style)/);
    expect(out).not.toMatch(/<a [^>]*href="(javascript|\/relative)/);
    expect(out).not.toMatch(/<img /);
    // A name the allow-list only inherits is no tag, and its text stays.
    expect(html2("<constructor a>hello</constructor> tail")).toContain("hello");
    // A picture loads only over https from a public host; a number-spelled loopback is still loopback.
    for (const src of ["http://example.com/a.png", "https://127.0.0.1/a.png", "https://2130706433/a.png", "https://192.168.1.2/a.png", "https://localhost/a.png", "https://[::1]/a.png", "https://router.local/a.png", "https://localhost./a.png", "https://printer.local./a.png"]) {
      expect(html2(`<img src="${src}" alt="x"> ![y](${src})`)).not.toMatch(/<img /);
    }
    // Nesting has a bound: past it an opening tag is text.
    expect(html2("<b>".repeat(5000) + "deep")).toContain("deep");
    // The phone's messages keep every tag as text.
    expect(html("<b>bold</b> <img src=\"https://example.com/a.png\">")).not.toMatch(/<strong|<img/);
  });
});

