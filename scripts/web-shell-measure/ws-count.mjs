#!/usr/bin/env node
// Counts the page's /ws frames for MEASURE_WS_COUNT_SECONDS: those it
// received by `type`, those it sent by `kind`, and the full redraws it asked
// for (`terminal_viewport` with `new_view`), the one request every build's
// shell sends for a whole frame of a pane. A build whose daemon asks the
// pane for a redraw on its own logs that in its diagnostics instead
// (run.sh reads both). Connect before the page navigates away: the counts
// follow the tab across navigations.
import { connectPage } from "./cdp.mjs";

import { pathToFileURL } from "node:url";
import fs from "node:fs";

export function observeFrames(page) {
  const received = {};
  const sent = {};
  const receivedBytes = {};
  const sectionBytes = {};
  let firstSnapshot = null;
  let fullRedrawsAsked = 0;
  const tally = (into, key, data) => {
    const name = key.exec(data ?? "")?.[1] ?? "binary";
    into[name] = (into[name] ?? 0) + 1;
  };
  page.on("Network.webSocketFrameReceived", ({ response }) => {
    const data = response?.payloadData ?? "";
    const bytes = response?.opcode === 2 ? Buffer.from(data, "base64").length : Buffer.byteLength(data);
    const name = /"type":"([a-z_]+)"/.exec(data)?.[1] ?? "binary";
    tally(received, /"type":"([a-z_]+)"/, data);
    receivedBytes[name] = (receivedBytes[name] ?? 0) + bytes;
    if (response?.opcode === 2) return;
    let frame;
    try { frame = JSON.parse(data); } catch { return; }
    if (!["snapshot", "delta"].includes(frame.type)) return;
    for (const [section, value] of Object.entries(frame.payload ?? {})) {
      if (section !== "rest") {
        const key = `${frame.type}.${section}`;
        sectionBytes[key] = (sectionBytes[key] ?? 0) + Buffer.byteLength(JSON.stringify(value));
        continue;
      }
      for (const [field, body] of Object.entries(value ?? {})) {
        for (const [leaf, content] of ["navigator", "status", "terminal"].includes(field)
          ? Object.entries(body ?? {}) : [["", body]]) {
          const key = `${frame.type}.rest.${field}${leaf ? `.${leaf}` : ""}`;
          sectionBytes[key] = (sectionBytes[key] ?? 0) + Buffer.byteLength(JSON.stringify(content));
        }
      }
    }
    if (frame.type === "snapshot" && !firstSnapshot) {
      const rest = frame.payload?.rest ?? {};
      const workspaces = rest.navigator?.workspaces ?? [];
      const remote = rest.status?.remote ?? [];
      firstSnapshot = {
        bytes, workspaces: workspaces.length,
        checkouts: workspaces.reduce((n, row) => n + (row.checkouts?.length ?? 0), 0),
        branches: workspaces.reduce((n, row) => n + (row.branches?.length ?? 0), 0),
        pull_requests: workspaces.reduce((n, row) => n + (row.pull_requests?.length ?? 0), 0),
        agents: rest.navigator?.agents?.length ?? 0,
        remote_sessions: remote.filter(row => row.session).length,
        remote_workspaces: remote.reduce((n, row) => n + (row.session?.workspaces?.length ?? 0), 0),
      };
    }
  });
  page.on("Network.webSocketFrameSent", ({ response }) => {
    const data = response?.payloadData ?? "";
    tally(sent, /"kind":"([a-z_]+)"/, data);
    if (data.includes('"kind":"terminal_viewport"') && data.includes('"new_view":true')) fullRedrawsAsked += 1;
  });
  return {
    report: () => ({ received_by_type: { ...received }, received_bytes_by_type: { ...receivedBytes }, section_bytes: { ...sectionBytes }, first_snapshot: firstSnapshot,
      sent_by_kind: { ...sent }, full_redraws_asked: fullRedrawsAsked }),
    reset() {
      for (const counter of [received, receivedBytes, sectionBytes, sent]) {
        for (const key of Object.keys(counter)) delete counter[key];
      }
      fullRedrawsAsked = 0;
      firstSnapshot = null;
    },
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const args = process.argv.slice(2);
  if (args.length && (args.length !== 2 || args[0] !== "--ready-file")) throw new Error("usage: ws-count.mjs [--ready-file <new-path>]");
  const readyFile = args[1];
  if (readyFile && fs.existsSync(readyFile)) throw new Error("observer readiness file already exists");
  const cdpPort = process.env.MEASURE_CDP_PORT;
  const seconds = Number(process.env.MEASURE_WS_COUNT_SECONDS ?? 0);
  if (!cdpPort || !(seconds > 0)) throw new Error("MEASURE_CDP_PORT and MEASURE_WS_COUNT_SECONDS are required");
  const page = await connectPage(cdpPort, process.env.MEASURE_WS_REQUIRE_PROBE !== "0");
  try {
    const counter = observeFrames(page);
    await page.send("Network.enable");
    if (readyFile) {
      const temporary = `${readyFile}.${process.pid}.tmp`;
      fs.writeFileSync(temporary, JSON.stringify({ pid: process.pid, boundary: "Network.enable completed" }), { flag: "wx" });
      try { fs.linkSync(temporary, readyFile); } finally { fs.unlinkSync(temporary); }
    }
    await new Promise((resolve) => setTimeout(resolve, seconds * 1000));
    console.log(JSON.stringify({ method: "Existing page socket via CDP; UTF-8 text/application binary payload bytes, excludes framing/compression; sections count JSON value bytes", seconds, ...counter.report() }));
  } finally {
    page.close();
  }
}
