// Draws what a holder test hands it on headless xterms, as the web shell
// draws a pane (scrollback 0, Unicode 11 widths), and says what each shows.
//
// Reads one JSON document on standard input:
//   { "terminals": [ { "cols": n, "rows": n, "steps": [ step, ... ],
//                      "defaults": { "fg": "#rrggbb", "bg": "#rrggbb" } }, ... ] }
// where a step is { "write": "<hex>" } or { "resize": [cols, rows] }, and
// writes one JSON document on standard output:
//   { "screens": [ { "lines": [...], "wrapped": [...], "styles": [...], "x": n, "y": n,
//                    "cursorVisible": bool,
//                    "cursorStyle": "default"|"block"|"underline"|"bar",
//                    "cursorBlink": bool|null, "buffer": "normal"|"alternate",
//                    "data": "<hex>", "mouse": "none"|... }, ... ] }
// `data` is everything the terminal answered or reported, which a screen
// fragment must never cause. `styles` is each row's cells as runs of how
// they look: the colors shown (after reverse video; a default color as the
// pane's default `defaults` names, which a holder writes out when it
// resolves reverse video), the attributes, and the hyperlink.
import headless from "@xterm/headless";
import unicode11 from "@xterm/addon-unicode11";

const { Terminal } = headless;
const { Unicode11Addon } = unicode11;

const input = await new Promise((resolve, reject) => {
  const chunks = [];
  process.stdin.on("data", (chunk) => chunks.push(chunk));
  process.stdin.on("end", () => resolve(Buffer.concat(chunks).toString("utf8")));
  process.stdin.on("error", reject);
});
const { terminals } = JSON.parse(input);

const write = (terminal, bytes) =>
  new Promise((resolve) => terminal.write(bytes, resolve));

const hexColor = (value) => `#${value.toString(16).padStart(6, "0")}`;

// How a cell looks, as one comparable string.
const look = (terminal, cell, defaults) => {
  const color = (isDefault, isRgb, value, fallback) =>
    isDefault ? fallback : isRgb ? hexColor(value) : `p${value}`;
  let fg = color(cell.isFgDefault(), cell.isFgRGB(), cell.getFgColor(), defaults.fg);
  let bg = color(cell.isBgDefault(), cell.isBgRGB(), cell.getBgColor(), defaults.bg);
  if (cell.isInverse()) {
    [fg, bg] = [bg, fg];
  }
  // The underline's style and the hyperlink are kept in xterm's core.
  const extended = cell.extended;
  const flags = [
    cell.isBold() ? "b" : "",
    cell.isDim() ? "d" : "",
    cell.isItalic() ? "i" : "",
    cell.isUnderline() ? `u${extended?.underlineStyle ?? 1}` : "",
    cell.isBlink() ? "k" : "",
    cell.isStrikethrough() ? "s" : "",
    cell.isOverline() ? "o" : "",
    cell.isInvisible() ? "h" : "",
  ].join("");
  const urlId = extended?.urlId ?? 0;
  const link = urlId ? terminal._core._oscLinkService.getLinkData(urlId)?.uri ?? "?" : "";
  return `${fg}/${bg}/${flags}${link ? ` ${link}` : ""}`;
};

const styles = (terminal, defaults) => {
  const buffer = terminal.buffer.active;
  const rows = [];
  for (let row = 0; row < terminal.rows; row += 1) {
    const line = buffer.getLine(buffer.viewportY + row);
    const runs = [];
    for (let x = 0; line && x < terminal.cols; x += 1) {
      const cell = line.getCell(x);
      if (!cell || cell.getWidth() === 0) {
        continue;
      }
      const key = look(terminal, cell, defaults);
      const last = runs[runs.length - 1];
      if (last && last.key === key) {
        last.count += 1;
      } else {
        runs.push({ key, count: 1 });
      }
    }
    rows.push(runs.map((run) => `${run.count}x ${run.key}`).join(" | "));
  }
  return rows;
};

const screens = [];
for (const spec of terminals) {
  const terminal = new Terminal({
    cols: spec.cols,
    rows: spec.rows,
    scrollback: 0,
    allowProposedApi: true,
  });
  terminal.loadAddon(new Unicode11Addon());
  terminal.unicode.activeVersion = "11";
  const data = [];
  terminal.onData((answer) => data.push(Buffer.from(answer, "binary")));
  terminal.onBinary((answer) => data.push(Buffer.from(answer, "binary")));
  for (const step of spec.steps) {
    if (step.resize) {
      terminal.resize(step.resize[0], step.resize[1]);
    } else {
      await write(terminal, Buffer.from(step.write, "hex"));
    }
  }
  const buffer = terminal.buffer.active;
  const lines = [];
  const wrapped = [];
  for (let row = 0; row < terminal.rows; row += 1) {
    const line = buffer.getLine(buffer.viewportY + row);
    lines.push(line?.translateToString(true) ?? "");
    wrapped.push(line?.isWrapped ?? false);
  }
  screens.push({
    lines,
    // Whether each row continues the one above it, a soft wrap: a holder's
    // read joins those rows into one line.
    wrapped,
    styles: styles(terminal, spec.defaults ?? { fg: "dfg", bg: "dbg" }),
    x: buffer.cursorX,
    y: buffer.cursorY,
    // The public API says neither whether DECTCEM hid the cursor nor the
    // shape DECSCUSR chose; xterm keeps both in its core.
    cursorVisible: !terminal._core.coreService.isCursorHidden,
    cursorStyle: terminal._core.coreService.decPrivateModes.cursorStyle ?? "default",
    cursorBlink: terminal._core.coreService.decPrivateModes.cursorBlink ?? null,
    buffer: buffer.type,
    data: Buffer.concat(data).toString("hex"),
    mouse: terminal.modes.mouseTrackingMode,
  });
  terminal.dispose();
}
process.stdout.write(JSON.stringify({ screens }));
