import { describe, expect, it } from "vitest";
import { addInput, answerPending, refusalOf } from "./skillsMcp";
import type { StoreAnswer } from "../snapshot";

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
