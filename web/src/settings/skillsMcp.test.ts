import { describe, expect, it } from "vitest";
import { addInput, answerPending, homePath, pathParts, refusalOf, rowLine, shownColumns } from "./skillsMcp";
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
    expect(refusalOf({ ...answer("refused", false), refused: { cause: "too_large", reason: "copy limit" } })).toBe("too_large");
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
    ["/Users/al/.grok/config.toml", "~/.grok/config.toml"],
    ["/Users/al", "~"],
    ["/Users/alice/.grok/config.toml", "/Users/alice/.grok/config.toml"],
    ["/srv/skills/notes", "/srv/skills/notes"],
  ])("%s reads as %s", (path, shown) => {
    expect(homePath(path, "/Users/al/")).toBe(shown);
  });
});

// A Windows machine reports its own paths, with `\` and a drive letter, and names them without case.
describe("a Windows path under home", () => {
  it.each([
    ["C:\\Users\\al\\.grok\\config.toml", "~\\.grok\\config.toml"],
    ["c:\\users\\al\\.codex\\config.toml", "~\\.codex\\config.toml"],
    ["C:\\Users\\al", "~"],
    ["C:\\Users\\alice\\.grok\\config.toml", "C:\\Users\\alice\\.grok\\config.toml"],
    ["D:\\skills\\notes", "D:\\skills\\notes"],
  ])("%s reads as %s", (path, shown) => {
    expect(homePath(path, "C:\\Users\\al")).toBe(shown);
  });

  it("keeps the last name whole whichever separator it has", () => {
    expect(pathParts("~\\.grok\\config.toml")).toEqual({ head: "~\\.grok", tail: "\\config.toml" });
    expect(pathParts("~/.grok/config.toml")).toEqual({ head: "~/.grok", tail: "/config.toml" });
  });
});

describe("the line under a row's name", () => {
  const local = (command: string, args: string[]) => ({ source: { kind: "pasted" }, server: { reach: { kind: "local", command, args } } }) as Parameters<typeof rowLine>[0];

  it("is the description when there is one, never where the item comes from", () => {
    expect(rowLine({ description: "Draws architecture diagrams", source: { kind: "folder", path: "/Users/me/skills/archify" } } as Parameters<typeof rowLine>[0])).toBe("Draws architecture diagrams");
  });

  it("names a server by its package, image or host, and a skill by its repository or folder, never by a path", () => {
    expect(rowLine(local("npx", ["-y", "@upstash/context7-mcp"]))).toBe("@upstash/context7-mcp");
    expect(rowLine(local("pnpm", ["dlx", "@scope/server"]))).toBe("@scope/server");
    expect(rowLine(local("docker", ["run", "-i", "--rm", "ghcr.io/github/github-mcp-server"]))).toBe("ghcr.io/github/github-mcp-server");
    expect(rowLine(local("node", ["/Users/me/servers/files/index.js"]))).toBe("index.js");
    expect(rowLine(local("/usr/local/bin/my-server", []))).toBe("my-server");
    expect(rowLine(local("C:\\Tools\\server.exe", ["--stdio"]))).toBe("server.exe");
    expect(rowLine({ source: { kind: "pasted" }, server: { reach: { kind: "remote", url: "https://mcp.linear.app/sse", transport: "sse" } } } as Parameters<typeof rowLine>[0])).toBe("mcp.linear.app");
    expect(rowLine({ description: " ", source: { kind: "git", url: "https://github.com/acme/kit.git", path: "archify" } } as Parameters<typeof rowLine>[0])).toBe("acme/kit");
    expect(rowLine({ source: { kind: "folder", path: "/Users/me/skills/archify/" } } as Parameters<typeof rowLine>[0])).toBe("archify");
  });
});
