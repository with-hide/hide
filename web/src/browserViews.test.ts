import { describe, expect, it } from "vitest";
import { addressShown, addressUrl, browserDisplays, fileUrl, hostKey, isHtmlFile, parseWorkspaceKey, placements, stillFits, stillWanted, withoutClosed } from "./browserViews";
import type { ViewLayoutSnapshot } from "./snapshot";
import { workspaceKey } from "./viewLayout";

describe("the address field", () => {
  it("keeps a scheme, opens a path as a file, and picks http only for a loopback host", () => {
    expect(addressUrl("https://example.com/a?b#c")).toBe("https://example.com/a?b#c");
    expect(addressUrl("about:blank")).toBe("about:blank");
    expect(addressUrl("/Users/example/문서/a b.html")).toBe("file:///Users/example/%EB%AC%B8%EC%84%9C/a%20b.html");
    expect(addressUrl("localhost:5173")).toBe("http://localhost:5173");
    expect(addressUrl("127.0.0.1:8080/docs")).toBe("http://127.0.0.1:8080/docs");
    expect(addressUrl("  example.com ")).toBe("https://example.com");
    expect(addressUrl("two words")).toBeNull();
    expect(addressUrl("   ")).toBeNull();
  });

  it("shows a web address without its scheme and anything else whole", () => {
    expect(addressShown("http://127.0.0.1:3000/a.html")).toBe("127.0.0.1:3000/a.html");
    expect(addressShown("https://example.com/")).toBe("example.com/");
    expect(addressShown("file:///Users/example/a.html")).toBe("file:///Users/example/a.html");
    // What is shown loads the same page again.
    expect(addressUrl(addressShown("http://localhost:5173/x"))).toBe("http://localhost:5173/x");
  });

  it("writes a file URL the way the daemon writes the checked path back", () => {
    // hided/src/file_url.rs spells the same path this way.
    expect(fileUrl("/a/100%/#x?.html")).toBe("file:///a/100%25/%23x%3F.html");
    expect(fileUrl("C:/reports/a b.HTML")).toBe("file:///C:/reports/a%20b.HTML");
    expect(fileUrl("/a/{b}`c\"<d>")).toBe("file:///a/%7Bb%7D%60c%22%3Cd%3E");
  });
});

describe("placing pages", () => {
  const layout = {
    root: {
      split: {
        id: "s1",
        axis: "row",
        ratio: 0.5,
        first: { area: { id: "a1", active: "d2", displays: [{ id: "d1", kind: "file", path: "/r/a.ts" }, { id: "d2", kind: "browser", path: "", url: "https://a.test/", load: 7 }] } },
        second: { area: { id: "a2", active: "d3", displays: [{ id: "d3", kind: "browser", path: "", url: "http://localhost:3000/", load: 9 }] } },
      },
    },
  } as unknown as ViewLayoutSnapshot;

  it("names every browser display of the layout and where its page shows", () => {
    const rows = browserDisplays(layout);
    expect(rows).toEqual([
      { id: "d2", area_id: "a1", url: "https://a.test/", load: 7 },
      { id: "d3", area_id: "a2", url: "http://localhost:3000/", load: 9 },
    ]);
    const rect = { x: 10, y: 20, width: 300, height: 200 };
    expect(placements(rows, new Map([["d2", rect]]), new Set())).toEqual([
      { id: "d2", area_id: "a1", url: "https://a.test/", load: 7, rect, visible: true },
      { id: "d3", area_id: "a2", url: "http://localhost:3000/", load: 9, rect: null, visible: false },
    ]);
    // A still standing in its place keeps the page placed but not shown.
    expect(placements(rows, new Map([["d2", rect]]), new Set(["d2"]))[0]).toMatchObject({ rect, visible: false });
    // A page at a device size carries it beside its frame's rect; a page that fits carries none (PRD browser-width-zoom D-12).
    const sized = placements(rows, new Map([["d2", rect]]), new Set(), new Map([["d2", { width: 390, mobile: true }]]));
    expect(sized[0]).toMatchObject({ rect, device: { width: 390, mobile: true } });
    expect(sized[1]).not.toHaveProperty("device");
  });

  it("reads back the Workspace a host key names", () => {
    const workspace = { device_id: "local", path: "/Users/example/project" };
    expect(parseWorkspaceKey(workspaceKey(workspace))).toEqual(workspace);
    expect(parseWorkspaceKey("no separator")).toBeNull();
  });

  it("forgets what a closed display's page said, and nothing of another Workspace", () => {
    const front = workspaceKey({ device_id: "local", path: "/Users/example/a" });
    const other = workspaceKey({ device_id: "local", path: "/Users/example/b" });
    const entries = { [hostKey(front, "d1")]: 1, [hostKey(front, "d2")]: 2, [hostKey(other, "d2")]: 3 };
    expect(withoutClosed(entries, front, new Set(["d1"]))).toEqual({ [hostKey(front, "d1")]: 1, [hostKey(other, "d2")]: 3 });
    expect(withoutClosed(entries, front, new Set(["d1", "d2"]))).toBe(entries);
  });
});

it("offers Open in Browser only for an HTML file", () => {
  expect(["a.html", "b.HTM", "c.xhtml"].every(isHtmlFile)).toBe(true);
  expect(["a.md", "html", "a.html.txt"].some(isHtmlFile)).toBe(false);
});

describe("the idle still", () => {
  const rect = { x: 10, y: 20, width: 400.4, height: 300.6 };
  const cached = { url: "https://a.test/", width: 400, height: 301, still: "data:image/png;base64,AA" };

  it("draws a still only of the same address in a slot of the same size", () => {
    expect(stillFits(cached, "https://a.test/", rect)).toBe(true);
    expect(stillFits(cached, "https://a.test/b", rect)).toBe(false);
    expect(stillFits(cached, "https://a.test/", { ...rect, width: 420 })).toBe(false);
    expect(stillFits(undefined, "https://a.test/", rect)).toBe(false);
  });

  it("is taken again only when the page has none that fits, or its load finished, or it came into view", () => {
    expect(stillWanted(cached, "https://a.test/", rect, false)).toBe(false);
    expect(stillWanted(cached, "https://a.test/", rect, true)).toBe(true);
    expect(stillWanted(cached, "https://a.test/b", rect, false)).toBe(true);
    expect(stillWanted(cached, "https://a.test/", { ...rect, height: 200 }, false)).toBe(true);
    expect(stillWanted(undefined, "https://a.test/", rect, false)).toBe(true);
  });
});
