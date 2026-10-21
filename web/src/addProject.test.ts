// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { initializeInterfaceI18n } from "./i18n/instance";
import { addProjectHosts, alreadyRegistered, cloneUrlDraft, defaultProjectParent, hostWays, initialHost, parseCloneUrl, projectNameProblem, projectPath, refusalText, trimFolder } from "./addProject";
import type { Device, WorkspaceRegistration } from "./snapshot";

const { t } = initializeInterfaceI18n("en");
const ko = initializeInterfaceI18n("ko").t;

const device = (id: string, label: string, kind = "remote") => ({ id, label, kind }) as Device;
const registration = (path: string, device_id = "local") => ({ id: path, label: path, path, device_id, pinned: false }) as WorkspaceRegistration;

describe("Add a project", () => {
  it("lists this window's machine first, named for its system, then every registered device (PRD core-host-node-move B2)", () => {
    vi.stubGlobal("navigator", { platform: "MacIntel" });
    expect(addProjectHosts(undefined, t)).toEqual([{ id: "", label: "This Mac" }]);
    expect(addProjectHosts(undefined, ko)).toEqual([{ id: "", label: "이 Mac" }]);
    expect(addProjectHosts([device("mini", "Mini"), device("local", "Studio", "local")], t)).toEqual([
      { id: "local", label: "This Mac" },
      { id: "mini", label: "Mini" },
    ]);
    vi.stubGlobal("navigator", { platform: "Win32" });
    expect(addProjectHosts(undefined, t)).toEqual([{ id: "", label: "This PC" }]);
    vi.unstubAllGlobals();
  });

  describe("what each host offers (docs/UI_BEHAVIOR.md, Adding a project)", () => {
    afterEach(() => {
      window.location.hash = "";
      vi.unstubAllGlobals();
    });

    it("browses this window's own machine and makes a new project on the core's, both on one machine", () => {
      const devices = [device("core", "", "local"), device("mini", "Mini")];
      expect(hostWays("core", devices)).toEqual({ browse: true, otherWays: true, pickParent: true });
      expect(hostWays("mini", devices)).toEqual({ browse: false, otherWays: false, pickParent: false });
      // Before the snapshot names a machine, the empty id is the core's own, where the window runs.
      expect(hostWays("", undefined)).toEqual({ browse: true, otherWays: true, pickParent: true });
    });

    it("in a window on a node whose core runs elsewhere, browses the window's machine only, and makes a new project on the core's without its picker", () => {
      // The MacBook's window after the core moved to the mini: the core's row is the mini's, the MacBook dials in.
      vi.stubGlobal("navigator", { platform: "MacIntel" });
      window.location.hash = "#token=t&node=mbp";
      const devices = [{ ...device("mini-node", "Mac mini", "local") }, { ...device("mbp", "MacBook-Pro.local"), dials_in: true }];
      expect(hostWays("mbp", devices)).toEqual({ browse: true, otherWays: false, pickParent: false });
      expect(hostWays("mini-node", devices)).toEqual({ browse: false, otherWays: true, pickParent: false });
      expect(addProjectHosts(devices, t).map((host) => host.label)).toEqual(["This Mac", "Mac mini"]);
    });
  });

  it("opens on the focused device while it is listed, this Mac otherwise", () => {
    const hosts = addProjectHosts([device("mini", "Mini")], t);
    expect(initialHost(hosts, "mini")).toBe("mini");
    expect(initialHost(hosts, "gone")).toBe("");
    expect(initialHost(hosts, null)).toBe("");
  });

  it("sees a folder already registered on the same device, trailing slash or not", () => {
    const rows = [registration("/home/me/hide/"), registration("/home/me/app", "mini")];
    expect(alreadyRegistered(trimFolder("/home/me/hide/"), "local", rows)).toBe(true);
    expect(alreadyRegistered("/home/me/hide", "mini", rows)).toBe(false);
    expect(alreadyRegistered(trimFolder("/home/me/app//"), "mini", rows)).toBe(true);
    expect(alreadyRegistered("/home/me/other", "local", rows)).toBe(false);
  });

  it("names the folder a clone lands in the way Git does", () => {
    const named = (url: string) => {
      const parsed = parseCloneUrl(url);
      return parsed.ok ? [parsed.host, parsed.name] : parsed.reason;
    };
    expect(named("https://github.com/user/repo.git")).toEqual(["github.com", "repo"]);
    expect(named("https://user:token@GitHub.com:8443/org/repo/")).toEqual(["github.com", "repo"]);
    expect(named("git@github.com:user/repo.git")).toEqual(["github.com", "repo"]);
    expect(named("github.com:user/.dotfiles")).toEqual(["github.com", ".dotfiles"]);
    expect(named("ssh://git@example.com:2222/srv/repo.git")).toEqual(["example.com", "repo"]);
    expect(named("file:///tmp/fixtures/origin.git")).toEqual(["localhost", "origin"]);
  });

  it("refuses what Git would not clone or would read as something else", () => {
    for (const url of ["", "http://example.com/r.git", "git://example.com/r.git", "ext::sh -c id", "ext::true", "-uhttps://x/y", "https://github.com", "https://github.com/", "/home/me/repo", "repo", "git@github.com:", "git@github.com:.git", "file://relative/r"]) {
      expect(parseCloneUrl(url).ok, url).toBe(false);
    }
  });

  it("saves only complete clone URL drafts without user information, queries or fragments", () => {
    for (const raw of ["https://alice:password@example.test/repo.git", "https://token@example.test/repo.git", "https://example.test/repo.git?token=secret", "https://example.test/repo.git#secret", "git@example.test:repo.git", "https://alice:password@", "unfinished-secret"]) {
      expect(cloneUrlDraft(raw), raw).toBe("");
    }
    for (const raw of ["https://example.test/team/repo.git", "example.test:team/repo.git", "file:///work/repo.git"]) {
      expect(cloneUrlDraft(` ${raw} `)).toBe(raw);
    }
  });

  it("puts a new project where most of this Mac's projects live, the latest winning a tie, else in home", () => {
    expect(defaultProjectParent([], "local")).toBe("~");
    expect(defaultProjectParent([registration("/home/me/work/a"), registration("/home/me/side/b/"), registration("/home/me/x", "mini")], "local")).toBe("/home/me/side");
    // A worktree folder added last does not take the default away from where the projects are.
    expect(defaultProjectParent([registration("/home/me/a"), registration("/home/me/b"), registration("/home/me/app.worktrees/c")], "local")).toBe("/home/me");
    expect(defaultProjectParent([registration("/home/me/a", "mini")], "local")).toBe("~");
    // The device's Home is no project: a project added after it is not expected beside `~/hide`.
    expect(defaultProjectParent([registration("/home/me/work/a"), { ...registration("/home/me/hide"), home: true }], "local")).toBe("/home/me/work");
  });

  it("takes one folder name, and says why another is not one", () => {
    expect(projectNameProblem("my-project")).toBeNull();
    expect(projectNameProblem("")).toBeNull();
    expect(projectNameProblem(".hidden")).toBeNull();
    expect(projectNameProblem("a/b")).toBe("addProject.name.slash");
    expect(projectNameProblem(".")).toBe("addProject.name.dots");
    expect(projectNameProblem("..")).toBe("addProject.name.dots");
    expect(t("addProject.name.slash")).toBe("A name is one folder, without `/`.");
    expect(ko("addProject.name.dots")).toBe("`.`와 `..`는 폴더 이름이 될 수 없습니다.");
  });

  it("words a refusal in the operator's language, and a code it does not know with that code", () => {
    expect(refusalText("outside_home", t)).toBe("Only a folder inside your home folder can be added.");
    expect(refusalText("outside_home", ko)).toBe("홈 폴더 안의 폴더만 추가할 수 있습니다.");
    expect(refusalText("quota", ko)).toBe("폴더가 거부되었습니다 (quota).");
  });

  it("previews the full path as the name is typed", () => {
    expect(projectPath("/home/me/work", "")).toBe("/home/me/work/project-name");
    expect(projectPath("/home/me/work", "app")).toBe("/home/me/work/app");
  });
});
