// Draws what a holder test hands it on headless xterms, as the web shell
// draws a pane (scrollback 0, Unicode 11 widths), and says what each shows.
//
// Reads one JSON document on standard input:
//   { "terminals": [ { "cols": n, "rows": n, "steps": [ step, ... ] }, ... ] }
// where a step is { "write": "<hex>" } or { "resize": [cols, rows] }, and
// writes one JSON document on standard output:
//   { "screens": [ { "lines": [...], "x": n, "y": n, "cursorVisible": bool,
//                    "cursorStyle": "default"|"block"|"underline"|"bar",
//                    "cursorBlink": bool|null, "buffer": "normal"|"alternate",
//                    "data": "<hex>", "mouse": "none"|... }, ... ] }
// `data` is everything the terminal answered or reported, which a screen
// fragment must never cause.
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
  for (let row = 0; row < terminal.rows; row += 1) {
    lines.push(buffer.getLine(buffer.viewportY + row)?.translateToString(true) ?? "");
  }
  screens.push({
    lines,
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
