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

  /** Each row of the drawn table as `tag[align]:text` cells. */
  const tableRows = (text: string, options: { html?: boolean } = {}) => {
    const out = renderToStaticMarkup(<>{renderMarkdown(text, options)}</>);
    return Array.from(out.matchAll(/<tr[^>]*>(.*?)<\/tr>/g), ([, row]) =>
      Array.from(row!.matchAll(/<(th|td) class="([^"]*)"[^>]*>(.*?)<\/\1>/g), ([, tag, classes, inner]) => {
        const align = classes!.split(" ").find((name) => ["text-left", "text-center", "text-right"].includes(name))?.slice(5);
        return `${tag}[${align}]:${inner}`;
      }),
    );
  };

  it("aligns each column as its delimiter mark says, header and body alike", () => {
    expect(tableRows("| a | b | c | d |\n|:---|:---:|---:|---|\n| 1 | 2 | 3 | 4 |")).toEqual([
      ["th[left]:a", "th[center]:b", "th[right]:c", "th[left]:d"],
      ["td[left]:1", "td[center]:2", "td[right]:3", "td[left]:4"],
    ]);
  });

  it("keeps an empty cell in its column, fills a short row and cuts a long one to the header's columns", () => {
    expect(tableRows("|  | b | c |\n|---|---|:-:|\n| 1 |  | 3 |\n| only |\n| x | y | z | extra |")).toEqual([
      ["th[left]:", "th[left]:b", "th[center]:c"],
      ["td[left]:1", "td[left]:", "td[center]:3"],
      ["td[left]:only", "td[left]:", "td[center]:"],
      ["td[left]:x", "td[left]:y", "td[center]:z"],
    ]);
  });

  it("reads an escaped pipe as text inside its cell and a row without outer pipes as the same row", () => {
    expect(tableRows("a | b\n--- | ---:\nx \\| y | z")).toEqual([
      ["th[left]:a", "th[right]:b"],
      ["td[left]:x | y", "td[right]:z"],
    ]);
  });

  it("draws a cell's emphasis, code and, with HTML on, its links and line breaks", () => {
    const [, row] = tableRows("| a | b |\n|---|---|\n| **굵게** `c` | [링크](https://example.com/x)<br>다음 |", { html: true });
    expect(row).toEqual([
      'td[left]:<strong class="font-semibold">굵게</strong> <code class="rounded-xs bg-secondary px-xxs font-mono">c</code>',
      expect.stringMatching(/^td\[left\]:<a href="https:\/\/example.com\/x"[^>]*>링크<\/a><br\/>다음$/),
    ]);
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

