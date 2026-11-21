// The Google Play screenshots, feature graphic and icon are made the same way (`bash mobile/scripts/build-android.sh
// store-shots`): the words and steps are the iOS shots' own (mobile/store/shots.json), and mobile/store/android/shots.json
// says which of them Play shows, at which size, and in which Play locale; the emulator half is `planAndroid`, the frame
// half `composeAndroid`.
//
// The App Store screenshots, made in two halves by one command (`bash mobile/scripts/build-ios.sh store-shots`):
//
//   1. On the build machine, the app in a simulator runs through each shot's steps in demo mode and the screen is captured
//      raw (`plan` below tells mini-ios.sh what to press, in which language, for how long);
//   2. Here, `compose` draws each shot's words and a device frame around the raw capture and writes the 1206x2622 PNG
//      with no alpha channel that App Store Connect takes.
//
// The words and the steps are `mobile/store/shots.json`. The app's screen in a shot is the capture, never redrawn: the
// frame and the headline are the only things this script makes. The notification shot is the same kind of capture as the
// others: the app in the simulator, with the banner iOS itself drew over it.
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
    if (!['screen', 'notification'].includes(shot.capture)) problems.push(`${shot.id}: capture is "${shot.capture}"`);
    if (!shot.steps.length) problems.push(`${shot.id}: no steps`);
    for (const language of languages) {
      const text = shot.text[language];
      if (!text?.headline || !text.support || !text.accent) { problems.push(`${shot.id}: ${language} has no headline, accent and support`); continue; }
      if (!Array.isArray(text.headline) || text.headline.length < 1 || text.headline.length > 2 || text.headline.some((line) => typeof line !== 'string' || !line)) { problems.push(`${shot.id}: ${language} headline is one or two lines, written as a list`); continue; }
      if (!text.headline.some((line) => line.includes(text.accent))) problems.push(`${shot.id}: ${language} accent "${text.accent}" is not inside one line of the headline`);
      for (const [field, value] of Object.entries({ ...text, headline: text.headline.join(' ') })) {
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

/** The unfiltered rows of an 8-bit, non-interlaced PNG of `channels` bytes per pixel, as one buffer. */
function pixelRows(png, info, channels) {
  const raw = inflateSync(Buffer.concat(chunks(png).filter((one) => one.type === 'IDAT').map((one) => one.data)));
  const stride = info.width * channels;
  const rows = Buffer.alloc(stride * info.height);
  for (let y = 0; y < info.height; y += 1) {
    const filter = raw[y * (stride + 1)];
    const line = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    for (let x = 0; x < stride; x += 1) {
      const left = x >= channels ? rows[y * stride + x - channels] : 0;
      const up = y > 0 ? rows[(y - 1) * stride + x] : 0;
      const corner = x >= channels && y > 0 ? rows[(y - 1) * stride + x - channels] : 0;
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
  return rows;
}

/** An 8-bit PNG of the given colour type (2 RGB, 6 RGBA) from its unfiltered rows. */
function encode(info, colorType, rows, channels) {
  const stride = info.width * channels;
  const out = Buffer.alloc((stride + 1) * info.height);
  for (let y = 0; y < info.height; y += 1) rows.copy(out, y * (stride + 1) + 1, y * stride, (y + 1) * stride);
  const header = Buffer.alloc(13);
  header.writeUInt32BE(info.width, 0); header.writeUInt32BE(info.height, 4); header[8] = 8; header[9] = colorType;
  return Buffer.concat([SIGNATURE, chunk('IHDR', header), chunk('IDAT', deflateSync(out, { level: 9 })), chunk('IEND', Buffer.alloc(0))]);
}

/** The same 8-bit RGBA picture as an RGB one. An alpha channel that is not fully opaque is an error: the page has no background to blend with. */
export function withoutAlpha(png) {
  const info = pngInfo(png);
  if (info.colorType === 2) return png;
  if (info.colorType !== 6 || info.bitDepth !== 8 || info.interlaced) throw new Error(`cannot flatten a PNG of colour type ${info.colorType}, depth ${info.bitDepth}, interlaced ${info.interlaced}`);
  const rows = pixelRows(png, info, 4);
  const flat = Buffer.alloc(info.width * info.height * 3);
  for (let at = 0, out = 0; at < rows.length; at += 4) {
    if (rows[at + 3] !== 255) throw new Error(`pixel ${(at / 4) % info.width},${Math.floor(at / 4 / info.width)} is not opaque`);
    flat[out++] = rows[at]; flat[out++] = rows[at + 1]; flat[out++] = rows[at + 2];
  }
  return encode(info, 2, flat, 3);
}

/** The same 8-bit RGB picture as a 32-bit one (an opaque alpha channel), which Play's icon is asked to be. */
export function withAlpha(png) {
  const info = pngInfo(png);
  if (info.colorType === 6) return png;
  if (info.colorType !== 2 || info.bitDepth !== 8 || info.interlaced) throw new Error(`cannot add alpha to a PNG of colour type ${info.colorType}, depth ${info.bitDepth}, interlaced ${info.interlaced}`);
  const rows = pixelRows(png, info, 3);
  const wide = Buffer.alloc(info.width * info.height * 4);
  for (let at = 0, out = 0; at < rows.length; at += 3) { wide[out++] = rows[at]; wide[out++] = rows[at + 1]; wide[out++] = rows[at + 2]; wide[out++] = 255; }
  return encode(info, 6, wide, 4);
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

/** The headline's lines are the ones shots.json wrote: the face's width never decides where a line ends, so an accent never splits. */
function headlineHtml({ headline, accent }) {
  return headline.map((line) => {
    const at = line.indexOf(accent);
    const marked = at < 0 ? escapeHtml(line) : `${escapeHtml(line.slice(0, at))}<mark>${escapeHtml(accent)}</mark>${escapeHtml(line.slice(at + accent.length))}`;
    return `<span class="line">${marked}</span>`;
  }).join('');
}

// A frame is laid out in canvas pixels. The phone is the capture scaled to `phoneWidth` inside a bezel; the iOS layout is
// the App Store's 1206-wide canvas, the Android one the same proportions on Play's 1080-wide one (corner radius of the
// device's screen, in capture pixels).
const IOS_LAYOUT = { phoneWidth: 920, bezel: 13, radius: 186, wordsTop: 150, wordsSide: 90, headline: 100, support: 48, gap: 36 };
const ANDROID_LAYOUT = { phoneWidth: 760, bezel: 12, radius: 110, wordsTop: 120, wordsSide: 70, headline: 90, support: 42, gap: 30 };

/** The page the browser draws for one shot. `capturePath` is the raw capture. */
export function frameHtml({ shot, language, tokens, capturePath, canvas, layout = IOS_LAYOUT }) {
  const text = shot.text[language];
  const PHONE_WIDTH = layout.phoneWidth;
  const BEZEL = layout.bezel;
  const screenWidth = PHONE_WIDTH - BEZEL * 2;
  const scale = screenWidth / canvas.width;
  const screenHeight = Math.round(canvas.height * scale);
  const radius = Math.round(layout.radius * scale);
  const capture = fileUrl(capturePath);
  const inner = `<img class="capture" src="${capture}">`;
  return `<!doctype html><html lang="${language}"><meta charset="utf-8"><style>
@font-face { font-family: Inter; src: url("${fileUrl(tokens.inter)}"); font-weight: 100 900; }
@font-face { font-family: Pretendard; src: url("${fileUrl(tokens.pretendard)}"); font-weight: 100 900; }
* { box-sizing: border-box; margin: 0; }
html, body { width: ${canvas.width}px; height: ${canvas.height}px; background: ${tokens.field}; overflow: hidden; }
body { font-family: Inter, Pretendard, system-ui, sans-serif; color: ${tokens.ink}; position: relative; }
.words { position: absolute; left: 0; right: 0; top: ${layout.wordsTop}px; padding: 0 ${layout.wordsSide}px; text-align: center; }
h1 .line { display: block; white-space: nowrap; }
h1 { font-size: ${layout.headline}px; line-height: 1.14; font-weight: 750; letter-spacing: -0.02em; }
h1 mark { background: ${tokens.accent}; color: ${tokens.ink}; padding: 0 0.16em; border-radius: 0.2em; -webkit-box-decoration-break: clone; box-decoration-break: clone; }
p { margin-top: ${layout.gap}px; font-size: ${layout.support}px; line-height: 1.3; font-weight: 500; color: ${tokens.muted}; text-wrap: balance; word-break: keep-all; }
.phone { position: absolute; left: ${(canvas.width - PHONE_WIDTH) / 2}px; top: ${canvas.height - screenHeight - BEZEL * 2 - 45}px; width: ${PHONE_WIDTH}px; height: ${screenHeight + BEZEL * 2}px; background: ${tokens.bezel}; border-radius: ${radius + BEZEL}px; padding: ${BEZEL}px; box-shadow: 0 30px 80px rgba(16, 17, 18, 0.28); }
.screen { position: relative; width: ${screenWidth}px; height: ${screenHeight}px; border-radius: ${radius}px; overflow: hidden; background: #fff; }
.capture { display: block; width: 100%; height: 100%; }
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
  let browser;
  const written = [];
  try {
    browser = await chromium.launch();
    const page = await browser.newPage({ viewport: shots.canvas, deviceScaleFactor: 1 });
    for (const language of Object.keys(shots.languages)) {
      await mkdir(join(outDir, language), { recursive: true });
      for (const shot of shots.shots) {
        const capturePath = resolve(rawDir, `${language}-${shot.file}.png`);
        const capture = await readFile(capturePath).catch(() => { throw new Error(`no capture for ${language} ${shot.file}: ${capturePath}`); });
        // The simulator's screen is the canvas: a capture of another size is a different device or a failed screenshot, never framed.
        const raw = pngInfo(capture);
        if (raw.width !== shots.canvas.width || raw.height !== shots.canvas.height) throw new Error(`the capture for ${language} ${shot.file} is ${raw.width}x${raw.height}, not ${shots.canvas.width}x${shots.canvas.height}: ${capturePath}`);
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
    await browser?.close();
    await rm(work, { recursive: true, force: true });
  }
  return written;
}

// ---- Google Play: the same shots on Android's sizes ----

export const loadAndroidShots = async (root = ROOT) => JSON.parse(await readFile(join(root, 'store/android/shots.json'), 'utf8'));

/** The Android list's shots as the iOS list wrote them (steps and words), in the Android list's order. */
export function androidShotsOf(android, shots) {
  return android.shots.map((id) => shots.shots.find((shot) => shot.id === id)).filter(Boolean);
}

/** What is wrong with the Play list, empty when nothing is. Play takes 2 to 8 phone screenshots; a page here has at least 4 per language. */
export function androidShotProblems(android, shots) {
  const problems = [];
  const { width, height } = android.canvas;
  // Play: each side 320 to 3840 px and no side more than twice the other; this list is 9:16 at 1080 wide so a phone frame fits.
  if (width !== 1080 || height !== 1920) problems.push(`the canvas is ${width}x${height}, the Android shots are 1080x1920`);
  if (android.featureGraphic.width !== 1024 || android.featureGraphic.height !== 500) problems.push(`the feature graphic is ${android.featureGraphic.width}x${android.featureGraphic.height}, Play's is 1024x500`);
  if (android.icon.size !== 512) problems.push(`the icon is ${android.icon.size}, Play's is 512x512`);
  if (new Set(android.shots).size !== android.shots.length) problems.push('a shot is listed twice');
  if (android.shots.length < 4 || android.shots.length > 8) problems.push(`${android.shots.length} shots: a Play page here has 4 to 8`);
  for (const id of [...android.shots, android.featureGraphic.shot]) {
    const shot = shots.shots.find((one) => one.id === id);
    if (!shot) { problems.push(`${id} is not a shot of mobile/store/shots.json`); continue; }
    // The Android app's notification is the system's own and is not part of the shots' capture, so a shot of one is not made.
    if (shot.capture !== 'screen') problems.push(`${id}: capture is "${shot.capture}", an Android shot is a screen`);
  }
  if (!android.shots.includes(android.featureGraphic.shot)) problems.push(`the feature graphic's shot ${android.featureGraphic.shot} is not one of the shots`);
  for (const [language, one] of Object.entries(android.languages)) {
    if (!shots.languages[language]) problems.push(`${language} is not a language of mobile/store/shots.json`);
    if (!one.tag || !one.folder) problems.push(`${language}: needs a tag and a folder`);
  }
  return problems;
}

/** The emulator half's plan, one tab-separated line per language and shot: language, file, settle seconds, locale tag, steps. */
export function planAndroid(android, shots, languages = Object.keys(android.languages)) {
  const lines = [];
  for (const language of languages) {
    for (const shot of androidShotsOf(android, shots)) {
      const steps = shot.steps.map((step) => step.replace('{reply}', shot.text[language].reply ?? '')).join(';;');
      lines.push([language, shot.file, shot.settleSeconds, android.languages[language].tag, steps].join('\t'));
    }
  }
  return lines;
}

const FEATURE_PHONE = 330;

/** The feature graphic: the headline and support line of its shot on the left, the phone on the right, cut by the bottom edge. */
export function featureHtml({ shot, language, tokens, capturePath, canvas, size }) {
  const text = shot.text[language];
  const bezel = 8;
  const screenWidth = FEATURE_PHONE - bezel * 2;
  const screenHeight = Math.round(canvas.height * screenWidth / canvas.width);
  return `<!doctype html><html lang="${language}"><meta charset="utf-8"><style>
@font-face { font-family: Inter; src: url("${fileUrl(tokens.inter)}"); font-weight: 100 900; }
@font-face { font-family: Pretendard; src: url("${fileUrl(tokens.pretendard)}"); font-weight: 100 900; }
* { box-sizing: border-box; margin: 0; }
html, body { width: ${size.width}px; height: ${size.height}px; background: ${tokens.field}; overflow: hidden; }
body { font-family: Inter, Pretendard, system-ui, sans-serif; color: ${tokens.ink}; position: relative; }
.words { position: absolute; left: 56px; top: 0; bottom: 0; width: 560px; display: flex; flex-direction: column; justify-content: center; }
h1 .line { display: block; white-space: nowrap; }
h1 { font-size: 52px; line-height: 1.14; font-weight: 750; letter-spacing: -0.02em; }
h1 mark { background: ${tokens.accent}; color: ${tokens.ink}; padding: 0 0.16em; border-radius: 0.2em; -webkit-box-decoration-break: clone; box-decoration-break: clone; }
p { margin-top: 20px; font-size: 24px; line-height: 1.3; font-weight: 500; color: ${tokens.muted}; word-break: keep-all; }
.phone { position: absolute; left: 660px; top: 56px; width: ${FEATURE_PHONE}px; height: ${screenHeight + bezel * 2}px; background: ${tokens.bezel}; border-radius: ${Math.round(110 * screenWidth / canvas.width) + bezel}px; padding: ${bezel}px; box-shadow: 0 20px 50px rgba(16, 17, 18, 0.28); }
.screen { width: ${screenWidth}px; height: ${screenHeight}px; border-radius: ${Math.round(110 * screenWidth / canvas.width)}px; overflow: hidden; background: #fff; }
.capture { display: block; width: 100%; height: 100%; }
</style><body>
<div class="words"><h1>${headlineHtml(text)}</h1><p>${escapeHtml(text.support)}</p></div>
<div class="phone"><div class="screen"><img class="capture" src="${fileUrl(capturePath)}"></div></div>
</body></html>`;
}

/** The icon: the iOS app's icon art, which is a full square of the Android icon's own background and owl, drawn at Play's size. */
export const iconHtml = ({ iconPath, size }) => `<!doctype html><meta charset="utf-8"><style>* { margin: 0 } html, body { width: ${size}px; height: ${size}px; overflow: hidden } img { display: block; width: ${size}px; height: ${size}px }</style><img src="${fileUrl(iconPath)}">`;

/**
 * Writes the Play layout fastlane's `supply` reads, `<out>/<locale>/images/{phoneScreenshots/<file>.png, featureGraphic.png, icon.png}`,
 * from the raw captures `<raw>/<language>-<file>.png`. Screenshots and the feature graphic are 24-bit PNGs, the icon a 32-bit one.
 */
export async function composeAndroid(rawDir, outDir, { repo = REPO, root = ROOT } = {}) {
  const [android, shots] = await Promise.all([loadAndroidShots(root), loadShots(root)]);
  const problems = androidShotProblems(android, shots);
  if (problems.length) throw new Error(`mobile/store/android/shots.json:\n  ${problems.join('\n  ')}`);
  const tokens = await frameTokens(repo);
  const { chromium } = await playwright(repo);
  const work = await mkdtemp(join(tmpdir(), 'store-shots-'));
  let browser;
  const written = [];
  const draw = async (page, name, html, size, flatten) => {
    const file = join(work, name);
    await writeFile(file, html);
    await page.setViewportSize(size);
    await page.goto(fileUrl(file));
    await page.evaluate(() => document.fonts.ready);
    const png = await page.screenshot({ type: 'png' });
    return flatten ? withoutAlpha(png) : withAlpha(png);
  };
  try {
    browser = await chromium.launch();
    const page = await browser.newPage({ viewport: android.canvas, deviceScaleFactor: 1 });
    const iconPath = resolve(root, android.icon.source);
    for (const [language, one] of Object.entries(android.languages)) {
      const images = join(outDir, one.folder, 'images');
      await mkdir(join(images, 'phoneScreenshots'), { recursive: true });
      const capturePathOf = async (shot) => {
        const capturePath = resolve(rawDir, `${language}-${shot.file}.png`);
        const capture = await readFile(capturePath).catch(() => { throw new Error(`no capture for ${language} ${shot.file}: ${capturePath}`); });
        // The emulator's screen is the canvas: a capture of another size is a different device or a failed screenshot, never framed.
        const raw = pngInfo(capture);
        if (raw.width !== android.canvas.width || raw.height !== android.canvas.height) throw new Error(`the capture for ${language} ${shot.file} is ${raw.width}x${raw.height}, not ${android.canvas.width}x${android.canvas.height}: ${capturePath}`);
        return capturePath;
      };
      const write = async (target, png, size, colorType) => {
        const info = pngInfo(png);
        if (info.width !== size.width || info.height !== size.height || info.colorType !== colorType) throw new Error(`${target} came out ${info.width}x${info.height}, colour type ${info.colorType}`);
        await writeFile(target, png);
        written.push(target);
      };
      for (const shot of androidShotsOf(android, shots)) {
        const html = frameHtml({ shot, language, tokens, capturePath: await capturePathOf(shot), canvas: android.canvas, layout: ANDROID_LAYOUT });
        await write(join(images, 'phoneScreenshots', `${shot.file}.png`), await draw(page, `${language}-${shot.file}.html`, html, android.canvas, true), android.canvas, 2);
      }
      const featured = shots.shots.find((shot) => shot.id === android.featureGraphic.shot);
      const feature = featureHtml({ shot: featured, language, tokens, capturePath: await capturePathOf(featured), canvas: android.canvas, size: android.featureGraphic });
      await write(join(images, 'featureGraphic.png'), await draw(page, `${language}-feature.html`, feature, android.featureGraphic, true), android.featureGraphic, 2);
      const icon = { width: android.icon.size, height: android.icon.size };
      await write(join(images, 'icon.png'), await draw(page, `${language}-icon.html`, iconHtml({ iconPath, size: android.icon.size }), icon, false), icon, 6);
    }
  } finally {
    await browser?.close();
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
  } else if (command === 'plan-android') {
    const [android, shots] = await Promise.all([loadAndroidShots(), loadShots()]);
    const problems = androidShotProblems(android, shots);
    if (problems.length) { process.stderr.write(`mobile/store/android/shots.json:\n  ${problems.join('\n  ')}\n`); process.exit(1); }
    process.stdout.write(`${planAndroid(android, shots, rest.length ? rest : undefined).join('\n')}\n`);
  } else if (command === 'compose-android' && rest.length === 2) {
    for (const file of await composeAndroid(resolve(rest[0]), resolve(rest[1]))) process.stdout.write(`${file}\n`);
  } else if (command === 'compose' && rest.length === 2) {
    for (const file of await compose(resolve(rest[0]), resolve(rest[1]))) process.stdout.write(`${file}\n`);
  } else {
    process.stderr.write('usage: store-shots.mjs plan [language...] | compose <raw capture folder> <output folder> | plan-android [language...] | compose-android <raw capture folder> <output folder>\n');
    process.exit(2);
  }
}
