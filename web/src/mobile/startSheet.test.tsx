// @vitest-environment jsdom
// The phone's start sheet (docs/UI_BEHAVIOR.md, The phone app): where over the text as words, the device, the project and
// the branch, each opening its own list; the project list gathers each device's projects once, a device that is not connected
// dimmed and not choosable, and past hided's cap ends with one line counting the places left out.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { StartSheet } from "./StartSheet";
import type { ServerFrame, StartTarget } from "./protocol";
import { applyFrame, patch, usePhone } from "./store";

vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
  // cmdk scrolls its highlighted row into view, which jsdom does not lay out.
  Element.prototype.scrollIntoView = () => undefined;
  globalThis.ResizeObserver ??= class { observe() {} unobserve() {} disconnect() {} } as unknown as typeof ResizeObserver;
});
const initial = usePhone.getState();
let root: Root | null = null;
beforeEach(() => vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true));
afterEach(async () => {
  if (root) await act(async () => root!.unmount());
  root = null;
  usePhone.setState(initial, true);
  vi.unstubAllGlobals();
});

const target = (id: string, project: string, branch: string | null, main: boolean, device = "local", label: string | null = null, connected = true): StartTarget =>
  ({ id, device_id: device, place: branch ? `${project} · ${branch}` : project, project, project_id: `${device}/${project}`, branch, main, device_label: label, connected });

const catalog = (targets: StartTarget[], omitted?: number): ServerFrame => ({
  type: "start_catalog",
  targets,
  core_platform: "windows x86_64",
  kinds: [{ id: "claude", models: ["opus"] }, { id: "codex", models: [] }],
  remembered: { kind: null, models: {} },
  ...(omitted === undefined ? {} : { omitted }),
});

const TARGETS = [
  target("c2", "hide", "prd/a-very-long-branch-name-that-goes-on", false),
  target("c1", "hide", "main", true),
  target("c3", "notes", null, true),
  target("rc1", "contong", "main", true, "mini", "mini"),
  target("wc1", "win-tests", "main", true, "window", "window", false),
];

async function open(frame: ServerFrame) {
  applyFrame(frame);
  patch({ connected: true, startSheet: { ...usePhone.getState().startSheet, open: true } });
  root = createRoot(document.createElement("div"));
  await act(async () => root!.render(<StartSheet />));
}

// The sheets draw into the document, not into the container.
const find = (selector: string) => document.body.querySelector<HTMLElement>(selector);
const all = (selector: string) => [...document.body.querySelectorAll<HTMLElement>(selector)];
const press = async (selector: string) => act(async () => find(selector)!.click());
const words = () => find('[data-phone-start-where="true"]')!.textContent;

it("puts where over the text: the machine named as its system says, the project, then the branch of its main checkout", async () => {
  await open(catalog(TARGETS));
  expect(words()).toBe("This PC" + "hide" + "›" + "main");
  const order = all("[data-phone-start-where], [data-phone-start-text], [data-phone-start-who]").map((node) => Object.keys(node.dataset)[0]);
  expect(order).toEqual(["phoneStartWhere", "phoneStartText", "phoneStartWho"]);
});

it("lists each device's projects once with their branch counts, and leaves a disconnected device's projects unchoosable", async () => {
  await open(catalog(TARGETS, 3));
  await press('[data-phone-start-project]');
  const list = find('[data-phone-start-list="project"]')!;
  const groups = [...list.querySelectorAll<HTMLElement>("[data-phone-start-device-group]")].map((group) => group.getAttribute("data-phone-start-device-group"));
  expect(groups).toEqual(["local", "mini", "window"]);
  const rows = [...list.querySelectorAll<HTMLElement>("[data-phone-start-option]")];
  expect(rows.map((row) => row.textContent)).toEqual(["hide2 branches", "notes", "contong", "win-tests"]);
  expect(rows[0]!.querySelector("[data-phone-start-chosen]")).not.toBeNull();
  expect(rows.at(-1)!.getAttribute("data-disabled")).toBe("true");
  expect(list.textContent).toContain("window · not connected");
  expect(list.querySelector("[data-phone-start-targets-omitted]")?.textContent).toBe("3 more");

  // Choosing another device's project starts on its main checkout, and the where line names that device.
  await act(async () => rows[2]!.click());
  expect(find('[data-phone-start-list="project"]')).toBeNull();
  expect(find("[data-phone-start-target]")!.getAttribute("data-phone-start-target")).toBe("rc1");
  expect(words()).toBe("mini" + "contong" + "›" + "main");
});

it("lists only the chosen project's checkouts in the branch list, its main one first and marked", async () => {
  await open(catalog(TARGETS));
  await press('[data-phone-start-branch]');
  const rows = all('[data-phone-start-list="branch"] [data-phone-start-option]');
  expect(rows.map((row) => row.textContent)).toEqual(["mainmain", "prd/a-very-long-branch-name-that-goes-on"]);
  await act(async () => rows[1]!.click());
  expect(find("[data-phone-start-branch]")!.getAttribute("data-phone-start-branch")).toBe("c2");
});

it("draws no branch for a folder, and an older Mac's places as projects of their own", async () => {
  await open(catalog([target("c3", "notes", null, true)]));
  expect(find("[data-phone-start-branch]")).toBeNull();
  await act(async () => root!.unmount());
  root = null;
  usePhone.setState(initial, true);
  await open(catalog([{ id: "c1", device_id: "local", place: "hide · main", device_label: null, connected: true }]));
  expect(words()).toBe("This PC" + "hide · main");
});

it("puts who under the text: the agent, then the model with the CLI's default first", async () => {
  await open(catalog(TARGETS));
  expect(find('[data-phone-start-who="true"]')!.textContent).toBe("Claude" + "CLI default");
  await press("[data-phone-start-model]");
  expect(all('[data-phone-start-list="model"] [data-phone-start-option]').map((row) => row.textContent)).toEqual(["CLI default", "opus"]);
});
