#!/usr/bin/env node
// Lost and reordered keys (PRD core-host-node-terminal D-08): types
// MEASURE_KEY_COUNT keys (10,000 by default) into the web shell's terminal
// through the shell's own key path, one CDP `Input.dispatchKeyEvent` per
// key as key-echo.mjs does, then reads the pane's content back from its
// receipt reader and counts the keys that never arrived and the ones that
// arrived out of order. It does not stop at the first loss: every count is
// reported.
//
// The keys spell distinct markers (`mNNNNN` and a space, Enter after every
// 100th), so a lost or moved key also breaks a marker. The pane runs
// `stty -echo -icanon; tee <receipt>`, a local byte receipt, so the reader
// returns what the pane received. MEASURE_PANE_READ is the shell command
// that prints it (run.sh: `cat <receipt>`).
import { performance } from "node:perf_hooks";
import { spawnSync } from "node:child_process";
import { connectPage } from "./cdp.mjs";

const cdpPort = process.env.MEASURE_CDP_PORT;
const paneId = process.env.MEASURE_PANE_ID;
const screenPane = process.env.MEASURE_SCREEN_PANE_ID ?? paneId;
const count = Number(process.env.MEASURE_KEY_COUNT ?? 10_000);
const intervalMs = Number(process.env.MEASURE_KEY_INTERVAL_MS ?? 0);
const perLine = 100;
if (!cdpPort || !paneId || !process.env.MEASURE_PANE_READ) {
  throw new Error("MEASURE_CDP_PORT, MEASURE_PANE_ID and MEASURE_PANE_READ are required");
}
if (!(count > 0 && count <= 99_999)) throw new Error("MEASURE_KEY_COUNT must be 1..99999");
const readCommand = process.env.MEASURE_PANE_READ;

const marker = (index) => `m${String(index).padStart(5, "0")}`;
// The typed keys, in order: markers until `count` keys, Enter after every
// 100th marker, and `#` as the last key, which a read of the pane shows
// (a trailing Enter would read as nothing).
const typed = [];
for (let index = 0; typed.length < count; index += 1) {
  for (const character of marker(index)) typed.push(character);
  typed.push((index + 1) % perLine === 0 ? "\r" : " ");
}
typed.length = count;
typed[count - 1] = "#";
const readPane = () => {
  const read = spawnSync("/bin/sh", ["-c", readCommand], { encoding: "utf8", maxBuffer: 64 * 1024 * 1024, timeout: 60_000 });
  if (read.status !== 0) throw new Error(`pane read failed (${read.status}): ${read.stderr}`);
  return read.stdout;
};

const page = await connectPage(cdpPort);
const focused = await page.evaluate(`(() => {
  const view = document.querySelector(${JSON.stringify(`[data-pane-view="${screenPane}"]`)});
  const input = view?.querySelector("textarea");
  input?.focus();
  return document.activeElement === input && Boolean(input);
})()`);
if (!focused) throw new Error(`the terminal of ${screenPane} did not take keyboard focus`);
if (/m\d{5}/.test(readPane())) throw new Error("the pane already shows markers; reset it first");

const load_before = spawnSync("uptime", { encoding: "utf8" }).stdout.trim();
const started = performance.now();
for (const character of typed) {
  if (character === "\r") {
    await page.send("Input.dispatchKeyEvent", { type: "keyDown", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13, text: "\r" });
  } else {
    await page.send("Input.dispatchKeyEvent", { type: "keyDown", text: character, unmodifiedText: character, key: character });
  }
  if (intervalMs > 0) await new Promise((resolve) => setTimeout(resolve, intervalMs));
}
const typed_ms = performance.now() - started;
page.close();

// Wait for the last marker to land, then read once more after a quiet spell.
const sent = typed.join("").replaceAll("\r", "\n");
const lastMarker = sent.match(/m\d{5}/g).pop();
const deadline = Date.now() + 60_000;
let content = readPane();
while (!(content.includes(lastMarker) && content.trimEnd().endsWith("#")) && Date.now() < deadline) {
  await new Promise((resolve) => setTimeout(resolve, 100));
  content = readPane();
}
// When the last key was seen, within one pane read: how fast keys drained.
const arrived_ms = performance.now() - started;
await new Promise((resolve) => setTimeout(resolve, 2000));
content = readPane();

// What the pane received: from the first marker on, its logical lines.
const start = content.indexOf(marker(0));
const received = start < 0 ? "" : content.slice(start).replace(/\s+$/, "");
const exact = received === sent;
// Keys: the longest common subsequence of sent and received keys is what
// arrived in order; the rest of the sent keys were lost or moved, the rest
// of the received ones moved or never typed.
const lcs = (a, b) => {
  let previous = new Uint32Array(b.length + 1);
  let current = new Uint32Array(b.length + 1);
  for (let i = 1; i <= a.length; i += 1) {
    for (let j = 1; j <= b.length; j += 1) {
      current[j] = a[i - 1] === b[j - 1] ? previous[j - 1] + 1 : Math.max(previous[j], current[j - 1]);
    }
    [previous, current] = [current, previous];
  }
  return previous[b.length];
};
const inOrder = exact ? sent.length : lcs(sent, received);
const counts = (text) => {
  const all = new Map();
  for (const character of text) all.set(character, (all.get(character) ?? 0) + 1);
  return all;
};
// A key that arrived but out of order is in both multisets yet not in the
// common subsequence; a lost key is missing from the received multiset.
const sentCounts = counts(sent);
const receivedCounts = counts(received);
let arrived = 0;
for (const [character, number] of sentCounts) arrived += Math.min(number, receivedCounts.get(character) ?? 0);
const lostKeys = sent.length - arrived;
const reorderedKeys = arrived - inOrder;
const unexpectedKeys = received.length - arrived;
// Markers, for a reader: which ones are missing or out of place.
let firstDifference = 0;
while (firstDifference < sent.length && sent[firstDifference] === received[firstDifference]) firstDifference += 1;
const around = (text) => JSON.stringify(text.slice(Math.max(0, firstDifference - 16), firstDifference + 16));
const seen = [...received.matchAll(/m(\d{5})/g)].map((match) => Number(match[1]));
const markerCount = sent.match(/m\d{5}/g).length;
const seenSet = new Set(seen);
const missingMarkers = [];
for (let index = 0; index < markerCount; index += 1) if (!seenSet.has(index)) missingMarkers.push(index);
console.log(JSON.stringify({
  method: "one CDP Input.dispatchKeyEvent per key into the measured pane's terminal; pane runs stty -echo -icanon with cat or tee; content read back through the configured receipt reader from the first marker on; lost = sent keys absent from what arrived; reordered = arrived keys outside the longest in-order common subsequence; unexpected = arrived keys never typed",
  pane_id: paneId,
  screen_pane_id: screenPane,
  keys_typed: sent.length,
  interval_ms: intervalMs,
  typed_ms: Math.round(typed_ms),
  arrived_ms: Math.round(arrived_ms),
  exact,
  keys_lost: lostKeys,
  keys_reordered: reorderedKeys,
  keys_unexpected: unexpectedKeys,
  first_difference: exact ? null : { key: firstDifference, sent: around(sent), received: around(received) },
  markers: markerCount,
  markers_missing: missingMarkers.length,
  markers_missing_first: missingMarkers.slice(0, 20),
  load_before,
  load_after: spawnSync("uptime", { encoding: "utf8" }).stdout.trim(),
}, null, 2));
