// The App Store screenshots, made in two halves by one command (`bash mobile/scripts/build-ios.sh store-shots`):
//
//   1. On the build machine, the app in a simulator runs through each shot's steps in demo mode and the screen is captured
//      raw (`plan` below tells mini-ios.sh what to press, in which language, for how long);
//   2. Here, `compose` draws each shot's words and a device frame around the raw capture and writes the 1206x2622 PNG
//      with no alpha channel that App Store Connect takes.
//
// The words and the steps are `mobile/store/shots.json`. The app's screen in a shot is the capture, never redrawn: the
// frame and the headline are the only things this script makes. A lock-screen shot ("banner") puts the simulator's own
// notification banner, cropped out of its capture, on a drawn lock screen: only the wallpaper and the clock are drawn.
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { deflateSync, inflateSync } from 'node:zlib';
import { thirdPartyNamesIn } from './store-limits.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const REPO = resolve(ROOT, '..');

export const loadShots = async (root = ROOT) => JSON.parse(await readFile(join(root, 'store/shots.json'), 'utf8'));

/** What is wrong with the shot list, empty when nothing is. */
export function shotProblems(shots) {
  const problems = [];
  const { width, height } = shots.canvas;
  if (width !== 1206 || height !== 2622) problems.push(`the canvas is ${width}x${height}, App Store Connect's required iPhone size is 1206x2622`);
  const languages = Object.keys(shots.languages);
  if (shots.shots.length < 3 || shots.shots.length > 10) problems.push(`${shots.shots.length} shots: App Store Connect takes 1 to 10 and a page needs at least 3`);
  const files = new Set();
  for (const shot of shots.shots) {
    if (files.has(shot.file)) problems.push(`${shot.file} is used by two shots`);
    files.add(shot.file);
    if (!['screen', 'banner'].includes(shot.capture)) problems.push(`${shot.id}: capture is "${shot.capture}"`);
    if (!shot.steps.length) problems.push(`${shot.id}: no steps`);
    if (shot.capture === 'banner' && !['x', 'y', 'width', 'height'].every((key) => Number.isInteger(shot.crop?.[key])) || (shot.capture === 'banner' && !shot.crop.placeAt)) problems.push(`${shot.id}: a banner shot names the banner's crop and where it sits`);
    for (const language of languages) {
      const text = shot.text[language];
      if (!text?.headline || !text.support || !text.accent) { problems.push(`${shot.id}: ${language} has no headline, accent and support`); continue; }
      if (!text.headline.includes(text.accent)) problems.push(`${shot.id}: ${language} accent "${text.accent}" is not in the headline`);
      for (const [field, value] of Object.entries(text)) {
        const names = thirdPartyNamesIn(value);
        if (names.length) problems.push(`${shot.id}: ${language} ${field} names ${names.join(', ')}`);
      }
      if (shot.steps.some((step) => step.includes('{reply}')) && !text.reply) problems.push(`${shot.id}: ${language} has no reply text`);
    }
  }
  return problems;
}

/** The simulator half's plan, one tab-separated line per language and shot: what mini-ios.sh runs. */
export function plan(shots, languages = Object.keys(shots.languages)) {
  const lines = [];
  for (const language of languages) {
    const { appleLanguage, appleLocale } = shots.languages[language];
    for (const shot of shots.shots) {
      const steps = shot.steps.map((step) => step.replace('{reply}', shot.text[language].reply ?? '')).join(';;');
      lines.push([language, shot.file, shot.capture, shot.settleSeconds, appleLanguage, appleLocale, steps].join('\t'));
    }
  }
  return lines;
}

// ---- PNG: the size of one, and the same picture without its alpha channel ----

const SIGNATURE = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
const crcTable = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
const crc32 = (buffer) => { let c = 0xffffffff; for (const byte of buffer) c = crcTable[(c ^ byte) & 0xff] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };

function chunks(png) {
  if (!png.subarray(0, 8).equals(SIGNATURE)) throw new Error('not a PNG');
  const found = [];
  for (let at = 8; at < png.length;) {
    const length = png.readUInt32BE(at);
    found.push({ type: png.toString('latin1', at + 4, at + 8), data: png.subarray(at + 8, at + 8 + length) });
    at += 12 + length;
  }
  return found;
}

const chunk = (type, data) => {
  const body = Buffer.concat([Buffer.from(type, 'latin1'), data]);
  const out = Buffer.alloc(12 + data.length);
  out.writeUInt32BE(data.length, 0);
  body.copy(out, 4);
  out.writeUInt32BE(crc32(body), 8 + data.length);
  return out;
};

/** @returns {{ width: number, height: number, bitDepth: number, colorType: number, interlaced: boolean }} */
export function pngInfo(png) {
  const header = chunks(png).find((one) => one.type === 'IHDR').data;
  return { width: header.readUInt32BE(0), height: header.readUInt32BE(4), bitDepth: header[8], colorType: header[9], interlaced: header[12] === 1 };
}

/** The same 8-bit RGBA picture as an RGB one. An alpha channel that is not fully opaque is an error: the page has no background to blend with. */
export function withoutAlpha(png) {
  const info = pngInfo(png);
  if (info.colorType === 2) return png;
  if (info.colorType !== 6 || info.bitDepth !== 8 || info.interlaced) throw new Error(`cannot flatten a PNG of colour type ${info.colorType}, depth ${info.bitDepth}, interlaced ${info.interlaced}`);
  const raw = inflateSync(Buffer.concat(chunks(png).filter((one) => one.type === 'IDAT').map((one) => one.data)));
  const stride = info.width * 4;
  const rows = Buffer.alloc(stride * info.height);
  for (let y = 0; y < info.height; y += 1) {
    const filter = raw[y * (stride + 1)];
    const line = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    for (let x = 0; x < stride; x += 1) {
      const left = x >= 4 ? rows[y * stride + x - 4] : 0;
      const up = y > 0 ? rows[(y - 1) * stride + x] : 0;
      const corner = x >= 4 && y > 0 ? rows[(y - 1) * stride + x - 4] : 0;
      let predicted = 0;
      if (filter === 1) predicted = left;
      else if (filter === 2) predicted = up;
      else if (filter === 3) predicted = (left + up) >> 1;
      else if (filter === 4) {
        const estimate = left + up - corner;
        const toLeft = Math.abs(estimate - left), toUp = Math.abs(estimate - up), toCorner = Math.abs(estimate - corner);
        predicted = toLeft <= toUp && toLeft <= toCorner ? left : toUp <= toCorner ? up : corner;
      } else if (filter !== 0) throw new Error(`unknown PNG filter ${filter}`);
      rows[y * stride + x] = (line[x] + predicted) & 0xff;
    }
  }
  const flat = Buffer.alloc((info.width * 3 + 1) * info.height);
  for (let y = 0; y < info.height; y += 1) {
    let out = y * (info.width * 3 + 1);
    flat[out++] = 0;
    for (let x = 0; x < info.width; x += 1) {
      const at = y * stride + x * 4;
      if (rows[at + 3] !== 255) throw new Error(`pixel ${x},${y} is not opaque`);
      flat[out++] = rows[at]; flat[out++] = rows[at + 1]; flat[out++] = rows[at + 2];
    }
  }
  const header = Buffer.alloc(13);
  header.writeUInt32BE(info.width, 0); header.writeUInt32BE(info.height, 4); header[8] = 8; header[9] = 2;
  return Buffer.concat([SIGNATURE, chunk('IHDR', header), chunk('IDAT', deflateSync(flat, { level: 9 })), chunk('IEND', Buffer.alloc(0))]);
}

// ---- The frame ----

const fileUrl = (path) => pathToFileURL(path).href;

/** The colours and faces the frame uses: design/tokens.json and the fonts the app ships. */
export async function frameTokens(repo = REPO) {
  const tokens = JSON.parse(await readFile(join(repo, 'design/tokens.json'), 'utf8')).tokens;
  return {
    field: tokens['--swatch-hide'].value,
    ink: tokens['--foreground'].hide.light,
    muted: tokens['--subtle-foreground'].hide.light,
    accent: tokens['--accent-choice-lime'].value,
    bezel: '#101112',
    inter: join(repo, 'web/src/assets/fonts/InterVariable.woff2'),
    pretendard: join(repo, 'web/src/assets/fonts/PretendardVariable.woff2'),
  };
}

const escapeHtml = (text) => text.replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);

function headlineHtml({ headline, accent }) {
  const at = headline.indexOf(accent);
  return `${escapeHtml(headline.slice(0, at))}<mark>${escapeHtml(accent)}</mark>${escapeHtml(headline.slice(at + accent.length))}`;
}

// A frame is laid out in canvas pixels. The phone is the capture scaled to PHONE_WIDTH inside a bezel.
const PHONE_WIDTH = 920;
const BEZEL = 13;

/** The lock screen a banner shot sits on: a flat night field, the clock and the date, and the banner's own pixels. */
function lockScreenHtml(shot, language, bannerUrl, scale, canvas) {
  const words = language === 'ko'
    ? { date: '11월 9일 월요일' }
    : { date: 'Monday, November 9' };
  const crop = shot.crop;
  return `<div class="lock">
    <div class="date">${words.date}</div>
    <div class="time">9:41</div>
    <div class="banner" style="left:${crop.placeAt.x * scale}px;top:${crop.placeAt.y * scale}px;width:${crop.width * scale}px;height:${crop.height * scale}px;background:url('${bannerUrl}') -${crop.x * scale}px -${crop.y * scale}px / ${canvas.width * scale}px auto no-repeat"></div>
  </div>`;
}

/** The page the browser draws for one shot. `capturePath` is the raw simulator capture. */
export function frameHtml({ shot, language, tokens, capturePath, canvas }) {
  const text = shot.text[language];
  const screenWidth = PHONE_WIDTH - BEZEL * 2;
  const scale = screenWidth / canvas.width;
  const screenHeight = Math.round(canvas.height * scale);
  const radius = Math.round(186 * scale);
  const capture = fileUrl(capturePath);
  const inner = shot.capture === 'banner'
    ? lockScreenHtml(shot, language, capture, scale, canvas)
    : `<img class="capture" src="${capture}">`;
  return `<!doctype html><html lang="${language}"><meta charset="utf-8"><style>
@font-face { font-family: Inter; src: url("${fileUrl(tokens.inter)}"); font-weight: 100 900; }
@font-face { font-family: Pretendard; src: url("${fileUrl(tokens.pretendard)}"); font-weight: 100 900; }
* { box-sizing: border-box; margin: 0; }
html, body { width: ${canvas.width}px; height: ${canvas.height}px; background: ${tokens.field}; overflow: hidden; }
body { font-family: Inter, Pretendard, system-ui, sans-serif; color: ${tokens.ink}; position: relative; }
.words { position: absolute; left: 0; right: 0; top: 150px; padding: 0 90px; text-align: center; }
h1 { font-size: 100px; line-height: 1.14; font-weight: 750; letter-spacing: -0.02em; text-wrap: balance; word-break: keep-all; }
h1 mark { background: ${tokens.accent}; color: ${tokens.ink}; padding: 0 0.16em; border-radius: 0.2em; -webkit-box-decoration-break: clone; box-decoration-break: clone; }
p { margin-top: 36px; font-size: 48px; line-height: 1.3; font-weight: 500; color: ${tokens.muted}; text-wrap: balance; word-break: keep-all; }
.phone { position: absolute; left: ${(canvas.width - PHONE_WIDTH) / 2}px; top: ${canvas.height - screenHeight - BEZEL * 2 - 45}px; width: ${PHONE_WIDTH}px; height: ${screenHeight + BEZEL * 2}px; background: ${tokens.bezel}; border-radius: ${radius + BEZEL}px; padding: ${BEZEL}px; box-shadow: 0 30px 80px rgba(16, 17, 18, 0.28); }
.screen { position: relative; width: ${screenWidth}px; height: ${screenHeight}px; border-radius: ${radius}px; overflow: hidden; background: #fff; }
.capture { display: block; width: 100%; height: 100%; }
.lock { position: absolute; inset: 0; background: #0D191B; color: #fff; text-align: center; }
.lock .date { position: absolute; top: ${380 * scale}px; left: 0; right: 0; font-size: ${54 * scale}px; font-weight: 600; opacity: .9; }
.lock .time { position: absolute; top: ${440 * scale}px; left: 0; right: 0; font-size: ${290 * scale}px; font-weight: 700; line-height: 1; letter-spacing: -0.03em; }
.lock .banner { position: absolute; border-radius: ${44 * scale}px; }
</style><body>
<div class="words"><h1>${headlineHtml(text)}</h1><p>${escapeHtml(text.support)}</p></div>
<div class="phone"><div class="screen">${inner}</div></div>
</body></html>`;
}

/** Playwright is the web shell's own dependency: it is found from there, not installed for this script. */
async function playwright(repo = REPO) {
  const require = createRequire(join(repo, 'web/package.json'));
  const loaded = await import(pathToFileURL(require.resolve('@playwright/test')).href);
  return loaded.chromium ? loaded : loaded.default;
}

/** Writes `<out>/<language>/<file>.png` for every shot that has a raw capture `<raw>/<language>-<file>.png`. */
export async function compose(rawDir, outDir, { repo = REPO, root = ROOT } = {}) {
  const shots = await loadShots(root);
  const problems = shotProblems(shots);
  if (problems.length) throw new Error(`mobile/store/shots.json:\n  ${problems.join('\n  ')}`);
  const tokens = await frameTokens(repo);
  const { chromium } = await playwright(repo);
  const work = await mkdtemp(join(tmpdir(), 'store-shots-'));
  const browser = await chromium.launch();
  const written = [];
  try {
    const page = await browser.newPage({ viewport: shots.canvas, deviceScaleFactor: 1 });
    for (const language of Object.keys(shots.languages)) {
      await mkdir(join(outDir, language), { recursive: true });
      for (const shot of shots.shots) {
        const capturePath = resolve(rawDir, `${language}-${shot.file}.png`);
        await readFile(capturePath).catch(() => { throw new Error(`no capture for ${language} ${shot.file}: ${capturePath}`); });
        const html = join(work, `${language}-${shot.file}.html`);
        await writeFile(html, frameHtml({ shot, language, tokens, capturePath, canvas: shots.canvas }));
        await page.goto(fileUrl(html));
        await page.evaluate(() => document.fonts.ready);
        const png = withoutAlpha(await page.screenshot({ type: 'png' }));
        const info = pngInfo(png);
        if (info.width !== shots.canvas.width || info.height !== shots.canvas.height || info.colorType !== 2) throw new Error(`${language} ${shot.file} came out ${info.width}x${info.height}, colour type ${info.colorType}`);
        const target = join(outDir, language, `${shot.file}.png`);
        await writeFile(target, png);
        written.push(target);
      }
    }
  } finally {
    await browser.close();
    await rm(work, { recursive: true, force: true });
  }
  return written;
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  const [command, ...rest] = process.argv.slice(2);
  if (command === 'plan') {
    const shots = await loadShots();
    const problems = shotProblems(shots);
    if (problems.length) { process.stderr.write(`mobile/store/shots.json:\n  ${problems.join('\n  ')}\n`); process.exit(1); }
    process.stdout.write(`${plan(shots, rest.length ? rest : undefined).join('\n')}\n`);
  } else if (command === 'compose' && rest.length === 2) {
    for (const file of await compose(resolve(rest[0]), resolve(rest[1]))) process.stdout.write(`${file}\n`);
  } else {
    process.stderr.write('usage: store-shots.mjs plan [language...] | compose <raw capture folder> <output folder>\n');
    process.exit(2);
  }
}
