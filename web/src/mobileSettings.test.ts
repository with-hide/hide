import { describe, expect, it } from "vitest";
import { initializeInterfaceI18n } from "./i18n/instance";
import { checklistView as checklistViewIn, codeCountdown, exposureLine as exposureLineIn, phoneLine as phoneLineIn, type MobileState, type PhoneRow } from "./mobileSettings";

// Korean is the wording Settings > Mobile shipped with; the rules read the same under it.
const t = initializeInterfaceI18n("ko").getFixedT(null, "translation");
const english = initializeInterfaceI18n("en").getFixedT(null, "translation");
const checklistView = (value: MobileState) => checklistViewIn(value, "이 Mac", t);
const exposureLine = (value: MobileState) => exposureLineIn(value, "이 Mac", t);
const phoneLine = (phone: PhoneRow, nowMs: number) => phoneLineIn(phone, nowMs, t, "ko");

const DAY = 24 * 60 * 60 * 1000;

function state(overrides: Partial<MobileState>): MobileState {
  return {
    enabled: true,
    exposure: "blocked",
    checklist: { installed: "ok", logged_in: "failed", https: "waiting", host_name: null },
    download_url: "https://tailscale.com/download",
    admin_url: "https://login.tailscale.com/admin/dns",
    foreign_target: null,
    failure: null,
    url: null,
    qr: null,
    code_expires_at_ms: null,
    phones: [],
    max_phones: 4,
    push_mode: "app_closed",
    now_ms: 0,
    ...overrides,
  };
}

describe("checklistView", () => {
  it("shows only the failing steps, each with its fix, and nothing for steps that passed or still wait", () => {
    const view = checklistView(state({}));
    expect(view.kind).toBe("failed");
    if (view.kind !== "failed") return;
    expect(view.steps.map((step) => step.id)).toEqual(["logged_in"]);
    expect(view.steps[0]?.action).toBeTruthy();
  });

  it("links the download page when Tailscale is missing and the admin console when HTTPS is off", () => {
    const missing = checklistView(state({ checklist: { installed: "failed", logged_in: "waiting", https: "waiting", host_name: null } }));
    const https = checklistView(state({ checklist: { installed: "ok", logged_in: "ok", https: "failed", host_name: "mac" } }));
    expect(missing.kind === "failed" && missing.steps[0]?.link?.href).toBe("https://tailscale.com/download");
    expect(https.kind === "failed" && https.steps.map((step) => [step.id, step.link?.href])).toEqual([["https", "https://login.tailscale.com/admin/dns"]]);
  });

  it("collapses to one ready line once every Mac step passed, whatever the QR is doing", () => {
    const passed = { installed: "ok", logged_in: "ok", https: "ok", host_name: "mac" } as const;
    expect(checklistView(state({ exposure: "exposed", checklist: passed })).kind).toBe("ready");
    expect(checklistView(state({ exposure: "exposed", qr: "https://mac.ts.net/m/#pair=x", checklist: passed })).kind).toBe("ready");
  });

  it("shows nothing while a step is still being read", () => {
    expect(checklistView(state({ checklist: { installed: "ok", logged_in: "waiting", https: "waiting", host_name: null } })).kind).toBe("pending");
    expect(checklistView(state({ checklist: null })).kind).toBe("pending");
  });
});

describe("the machine the phone pairs with", () => {
  // The core's machine, which runs Tailscale for the phone; a window on another machine names it rather than calling it this Mac.
  it("names the core's machine in the Tailscale steps and the serve entry line", () => {
    const missing = checklistViewIn(state({ checklist: { installed: "failed", logged_in: "waiting", https: "waiting", host_name: null } }), "Mac mini", english);
    expect(missing.kind === "failed" && missing.steps[0]?.title).toBe("Tailscale isn't installed on Mac mini");
    expect(exposureLineIn(state({ exposure: "foreign", foreign_target: "http://127.0.0.1:3000" }), "Mac mini", english)?.text).toBe("Not exposed · tailscale serve HTTPS 443 on Mac mini has an entry hide didn't create: http://127.0.0.1:3000. hide will not change it.");
    const here = checklistViewIn(state({ checklist: { installed: "failed", logged_in: "waiting", https: "waiting", host_name: null } }), "이 Mac", t);
    expect(here.kind === "failed" && here.steps[0]?.title).toBe("이 Mac에 Tailscale이 설치돼 있지 않아요");
  });
});

describe("exposureLine", () => {
  it("names the foreign entry hide will not touch and the failed serve command", () => {
    expect(exposureLine(state({ exposure: "foreign", foreign_target: "http://127.0.0.1:3000" }))?.text).toContain("http://127.0.0.1:3000");
    const failed = exposureLine(state({ exposure: "failed", failure: { step: "add", message: "serve config denied" } }));
    expect(failed?.tone).toBe("error");
    expect(failed?.text).toContain("serve config denied");
    expect(exposureLine(state({ exposure: "failed", failure: { step: "funnel", message: "Funnel을 끄면 이어집니다." } }))?.text).toContain("Funnel이 켜져 있어요");
  });

  it("is empty once exposed, with or without a QR, and while blocked on a Tailscale step", () => {
    expect(exposureLine(state({ exposure: "exposed", qr: "q" }))).toBeNull();
    expect(exposureLine(state({ exposure: "exposed" }))).toBeNull();
    expect(exposureLine(state({ exposure: "blocked" }))).toBeNull();
  });
});

describe("codeCountdown", () => {
  it("counts down from the daemon clock plus the time since the frame", () => {
    expect(codeCountdown(300_000, 0, 0)).toBe("5:00");
    expect(codeCountdown(300_000, 0, 61_000)).toBe("3:59");
    expect(codeCountdown(300_000, 0, 300_000)).toBeNull();
    expect(codeCountdown(null, 0, 0)).toBeNull();
  });
});

describe("phoneLine", () => {
  const phone: PhoneRow = { id: "p", name: "iPhone", last_seen_ms: 0, connected: false, notifications: "on", revoke_at_ms: 7 * DAY };

  it("reads 방금 while connected and carries the notification state", () => {
    expect(phoneLine({ ...phone, connected: true }, 3 * DAY)).toBe("방금 · 알림 받는 중");
  });

  it("counts the days to the seven-day revoke once away a day", () => {
    expect(phoneLine(phone, 3 * DAY)).toBe("3일 전 · 알림 받는 중 · 4일 뒤 자동 해지");
    expect(phoneLine({ ...phone, notifications: "unasked" }, 60 * 60 * 1000)).toBe("1시간 전");
  });
});

describe("in English", () => {
  const phone: PhoneRow = { id: "p", name: "iPhone", last_seen_ms: 0, connected: false, notifications: "on", revoke_at_ms: 7 * DAY };

  it("words the failing step, the foreign entry and the phone row without Korean", () => {
    const view = checklistViewIn(state({}), "This Mac", english);
    expect(view.kind === "failed" && view.steps.map((step) => step.title)).toEqual(["Not signed in to Tailscale"]);
    expect(exposureLineIn(state({ exposure: "foreign", foreign_target: null }), "This Mac", english)?.text).toBe(
      "Not exposed · tailscale serve HTTPS 443 on This Mac has an entry hide didn't create: Unknown target. hide will not change it.",
    );
    expect(phoneLineIn(phone, 3 * DAY, english, "en")).toBe("3 days ago · Receiving notifications · Automatically revoked in 4 days");
    expect(phoneLineIn({ ...phone, connected: true, notifications: "off" }, 3 * DAY, english, "en")).toBe("Just now · Notifications off");
  });
});

