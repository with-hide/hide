import { describe, expect, it } from "vitest";
import { externalLink } from "./externalLink";

describe("externalLink", () => {
  it("hands a web address and a mail link to the system, keeping only the scheme and host for the log", () => {
    expect(externalLink("https://sso.example.com/login?code=secret#x")).toEqual({ url: "https://sso.example.com/login?code=secret#x", protocol: "https:", host: "sso.example.com" });
    expect(externalLink("http://127.0.0.1:3000/a")).toEqual({ url: "http://127.0.0.1:3000/a", protocol: "http:", host: "127.0.0.1:3000" });
    expect(externalLink("mailto:a@example.com")).toMatchObject({ protocol: "mailto:" });
  });

  it("refuses a file, another app's scheme, a script and anything unparsable", () => {
    for (const url of ["file:///Users/example/a.html", "slack://open", "javascript:alert(1)", "data:text/html,x", "smb://share/x", "about:blank"]) {
      expect(externalLink(url)).toEqual({ refused: "protocol", protocol: new URL(url).protocol });
    }
    expect(externalLink("not a url")).toEqual({ refused: "unparsable" });
  });
});
