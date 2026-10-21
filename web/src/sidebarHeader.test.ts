import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { DeviceTitle, SidebarHeader } from "./components/sidebar-header";
import { TooltipProvider } from "./components/ui/tooltip";

const MENU = {
  devices: [
    { id: "local", label: "This Mac", remote: false, connected: true },
    { id: "mini", label: "mini", remote: true, connected: true },
  ],
  frontId: "local",
  onSelect: () => undefined,
  onAddDevice: () => undefined,
  onShowRail: () => undefined,
};

function title({ rail = true, name = { name: "This Mac", note: null as string | null } } = {}) {
  return renderToStaticMarkup(createElement(TooltipProvider, null, createElement(DeviceTitle, { rail, title: name, deviceMenu: MENU })));
}

function header(searchChord: string | null = "⌘K") {
  return renderToStaticMarkup(createElement(TooltipProvider, null, createElement(SidebarHeader, { searchChord, onSearch: () => undefined })));
}

/** The opening tag of the first element carrying `attribute`. */
function tag(html: string, attribute: string): string | null {
  return html.match(new RegExp(`<[a-z]+[^>]*\\s${attribute}(?=[\\s=>])[^>]*>`))?.[0] ?? null;
}

/** The text of every element carrying `attribute`, in document order. */
function texts(html: string, attribute: string): string[] {
  return [...html.matchAll(new RegExp(`<([a-z]+)[^>]*\\s${attribute}(?=[\\s=>])[^>]*>(.*?)</\\1>`, "g"))].map((match) => match[2]!.replace(/<[^>]+>/g, ""));
}

describe("the device title in the window's top row (PRD sidebar-titlebar-refresh B17, B21)", () => {
  it("names the device in front with its note", () => {
    const html = title({ name: { name: "mini", note: "Remote" } });
    expect(texts(html, "data-sidebar-title-name")).toEqual(["mini"]);
    expect(html).toContain("Remote");
  });

  it("turns the name into the device menu while the rail is hidden", () => {
    const html = title({ rail: false });
    expect(tag(html, "data-sidebar-device-menu")).toContain('aria-label="This Mac, switch device"');
    expect(texts(html, "data-sidebar-title-name")).toEqual(["This Mac"]);
    expect(title({ rail: true })).not.toContain("data-sidebar-device-menu");
  });
});

describe("the sidebar's Add project row (B51)", () => {
  const withAdd = (chord: string | null) =>
    renderToStaticMarkup(createElement(TooltipProvider, null, createElement(SidebarHeader, { searchChord: "⌘K", onSearch: () => undefined, addProject: { chord, onOpen: () => undefined } })));

  it("is one row with the word and its chord, drawn only where the host can pick a folder", () => {
    expect(texts(withAdd("⇧⌘N"), "data-sidebar-new-workspace")).toEqual(["Add project⇧⌘N"]);
    expect(header()).not.toContain("data-sidebar-new-workspace");
  });
});

describe("the sidebar's Search row (B12)", () => {
  it("is a visible row with the word Search and the chord as bound, and takes no text", () => {
    const html = header("⌃⇧K");
    expect(texts(html, "data-sidebar-search")).toEqual(["Search⌃⇧K"]);
    expect(tag(html, "data-sidebar-search-chord")).toContain('data-sidebar-search-chord="⌃⇧K"');
    expect(html).not.toContain("<input");
    expect(html).not.toContain("data-sidebar-new-workspace");
  });

  it("draws no chord when Search has none bound", () => {
    expect(header(null)).not.toContain("data-sidebar-search-chord");
  });
});
