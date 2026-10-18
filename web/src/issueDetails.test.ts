import { describe, expect, it } from "vitest";
import { issueDetailOf } from "./issueDetails";
import type { IssueDetail } from "./snapshot";

const project = { id: "project-on-node", device_id: "node" };
const key = "local:/repo#1";
const detail = (phase: IssueDetail["phase"]): IssueDetail => ({
  workspace_id: project.id, device_id: project.device_id, task_key: key, phase,
  body: phase === "ready" ? "Node's private memo" : null, message: null,
  labels: [], author: null, created_at_unix_ms: null, assignees: [], comment_count: null, comments: [],
});

describe("the active detail's project and device identity", () => {
  it.each(["reading", "ready", "failed"] as const)("reuses %s only for the exact project, device and task", (phase) => {
    const answer = detail(phase);
    expect(issueDetailOf(answer, project, key)).toBe(answer);
    expect(issueDetailOf(answer, { ...project, id: "project-on-core" }, key)).toBeNull();
    expect(issueDetailOf(answer, { ...project, device_id: "core" }, key)).toBeNull();
    expect(issueDetailOf(answer, project, "local:/repo#2")).toBeNull();
  });

  it("returns no body before a project is resolved or for an older unscoped snapshot", () => {
    expect(issueDetailOf(detail("ready"), null, key)).toBeNull();
    expect(issueDetailOf(undefined, project, key)).toBeNull();
    expect(issueDetailOf({ ...detail("ready"), workspace_id: undefined, device_id: undefined } as unknown as IssueDetail, project, key)).toBeNull();
  });
});
