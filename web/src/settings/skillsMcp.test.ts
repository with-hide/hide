import { describe, expect, it } from "vitest";
import { addInput, answerPending, homePath, refusalOf, shownColumns } from "./skillsMcp";
import type { StoreAnswer, StoreReport } from "../snapshot";

// What the one add field reads a pasted line as decides which flow opens:
// a skill source is read and picked from, a paste becomes an MCP server.
describe("the add field", () => {
  it.each([
    ["anthropics/skills", { kind: "git", url: "github.com/anthropics/skills" }],
    ["https://github.com/acme/archify/tree/main/archify", { kind: "git", url: "https://github.com/acme/archify/tree/main/archify" }],
    ["git@gitlab.com:acme/skills.git", { kind: "git", url: "git@gitlab.com:acme/skills.git" }],
    ["file:///srv/skills.git", { kind: "git", url: "file:///srv/skills.git" }],
    ["~/skills/aside-browser", { kind: "folder", path: "~/skills/aside-browser" }],
    ["/Users/me/skills", { kind: "folder", path: "/Users/me/skills" }],
    ["C:\\Users\\me\\skills", { kind: "folder", path: "C:\\Users\\me\\skills" }],
    ["npx -y @modelcontextprotocol/server-github", { kind: "mcp", text: "npx -y @modelcontextprotocol/server-github" }],
    ["https://mcp.linear.app/mcp", { kind: "mcp", text: "https://mcp.linear.app/mcp" }],
    ['{"mcpServers": {}}', { kind: "mcp", text: '{"mcpServers": {}}' }],
  ])("reads %s as its source", (text, read) => {
    expect(addInput(`  ${text}\n`)).toEqual(read);
  });

  it("reads an empty field as nothing to add", () => {
    expect(addInput("   ")).toBeNull();
  });
});

describe("a store request's answer", () => {
  const answer = (state: StoreAnswer["state"], refused: boolean): StoreAnswer => ({
    request_id: "r1",
    state,
    outcomes: [
      { change: { op: "check_updates" }, refused: null },
      { change: { op: "delete", item: "skill:gone" }, refused: refused ? { cause: "not_found", reason: "no item" } : null },
    ],
  }) as StoreAnswer;

  it("is pending from the press until the core answers", () => {
    expect(answerPending(null, undefined)).toBe(false);
    expect(answerPending("r1", undefined)).toBe(true);
    expect(answerPending("r1", answer("pending", false))).toBe(true);
    expect(answerPending("r1", answer("done", false))).toBe(false);
  });

  it("names the first change refused, or the request that never ran", () => {
    expect(refusalOf(answer("done", false))).toBeNull();
    expect(refusalOf(answer("done", true))).toBe("not_found");
    expect(refusalOf(answer("refused", false))).toBe("unavailable");
  });
});

// The shared skill folder is one column named 공용 while several shown
// agents read it, and that agent's own column when only one does (layout C).
describe("the shared skill folder's column", () => {
  const report = { columns: { skill: [{ id: "claude", agents: ["claude-code"], also: ["grok"] }, { id: "agents", agents: ["codex", "grok", "opencode"] }] } } as unknown as StoreReport;

  it("is shared while two of its agents are shown", () => {
    expect(shownColumns(report, "skill", new Set(["claude-code", "codex", "grok"]))).toEqual([
      { id: "claude", shared: false, agent: "claude-code", readers: ["claude-code", "grok"] },
      { id: "agents", shared: true, agent: null, readers: ["codex", "grok"] },
    ]);
  });

  it("is its one shown agent's column when only that agent reads it", () => {
    expect(shownColumns(report, "skill", new Set(["claude-code", "codex"]))[1]).toEqual({ id: "agents", shared: false, agent: "codex", readers: ["codex"] });
  });
});

// A path under the machine's home reads with ~; a sibling folder whose name
// starts the same is not under it.
describe("a path under home", () => {
  it.each([
    ["/Users/me/.grok/config.toml", "~/.grok/config.toml"],
    ["/Users/me", "~"],
    ["/Users/mel/.grok/config.toml", "/Users/mel/.grok/config.toml"],
    ["/srv/skills/notes", "/srv/skills/notes"],
  ])("%s reads as %s", (path, shown) => {
    expect(homePath(path, "/Users/me/")).toBe(shown);
  });
});
