import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { deflateSync, inflateSync } from 'node:zlib';
import { describe, expect, it } from 'vitest';
import { androidShotProblems, androidShotsOf, featureHtml, frameHtml, loadAndroidShots, loadShots, plan, planAndroid, pngInfo, shotProblems, withAlpha, withoutAlpha } from './store-shots.mjs';

const real = resolve(dirname(fileURLToPath(import.meta.url)), '..');

/** An RGBA PNG of 3x2 pixels drawn with every filter type a decoder meets, built here so the test needs no picture. */
function rgba(alpha = 255) {
  const pixels = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [10, 20, 30], [40, 50, 60], [70, 80, 90]];
  const width = 3;
  const rows = [];
  const filters = [0, 1];
  const raws = [0, 1].map((y) => Buffer.from(pixels.slice(y * width, y * width + width).flatMap(([r, g, b]) => [r, g, b, alpha])));
  rows.push(Buffer.concat([Buffer.from([filters[0]]), raws[0]]));
  // Row 2 uses the Sub filter: each byte minus the byte four to its left.
  const sub = Buffer.from(raws[1].map((value, at) => (value - (at >= 4 ? raws[1][at - 4] : 0)) & 0xff));
  rows.push(Buffer.concat([Buffer.from([filters[1]]), sub]));
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0); header.writeUInt32BE(2, 4); header[8] = 8; header[9] = 6;
  const crc = (buffer) => { let c = 0xffffffff; for (const byte of buffer) { c ^= byte; for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; } return (c ^ 0xffffffff) >>> 0; };
  const chunk = (type, data) => { const body = Buffer.concat([Buffer.from(type, 'latin1'), data]); const out = Buffer.alloc(12 + data.length); out.writeUInt32BE(data.length, 0); body.copy(out, 4); out.writeUInt32BE(crc(body), 8 + data.length); return out; };
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', header), chunk('IDAT', deflateSync(Buffer.concat(rows))), chunk('IEND', Buffer.alloc(0))]);
}

describe('the shot list as committed', () => {
  it('is a valid list for 1206x2622 shots in both languages', async () => {
    const shots = await loadShots(real);
    expect(shotProblems(shots)).toEqual([]);
    expect(Object.keys(shots.languages).sort()).toEqual(['en', 'ko']);
    expect(shots.shots.length).toBeGreaterThanOrEqual(3);
    // The first three show the agents of the person's computer on the phone: the list, a conversation, a notification.
    expect(shots.shots.slice(0, 3).map((shot) => shot.id)).toEqual(['agents', 'conversation', 'notification']);
  });

  it('plans one line per language and shot, with the reply words in the steps', async () => {
    const lines = plan(await loadShots(real), ['ko']).map((line) => line.split('\t'));
    expect(lines).toHaveLength(6);
    expect(lines[1]).toEqual(['ko', '02-conversation', 'screen', '12', 'ko', 'ko_KR', expect.stringContaining('[data-phone-reply]::=네, 3건 모두 반영해 줘.')]);
    expect(lines.every((line) => line.length === 7 && !line[6].includes('{reply}'))).toBe(true);
  });
});

describe('what is wrong with a shot list', () => {
  const base = async () => structuredClone(await loadShots(real));

  it('names the canvas, a missing language, a bad accent and another company', async () => {
    const shots = await base();
    shots.canvas = { width: 1290, height: 2796 };
    delete shots.shots[1].text.ko;
    shots.shots[0].text.en.accent = 'nowhere';
    shots.shots[2].text.en.headline = ['Reply to ChatGPT.', 'Know the moment.'];
    expect(shotProblems(shots)).toEqual(expect.arrayContaining([
      expect.stringContaining('1290x2796'),
      expect.stringContaining('conversation: ko has no headline'),
      expect.stringContaining('accent "nowhere"'),
      expect.stringContaining('names chatgpt'),
    ]));
  });

  it('refuses a headline that is not one or two written lines, and an accent that is not inside one of them', async () => {
    const shots = await base();
    shots.shots[0].text.en.headline = 'Answer your agents. Away from your desk.';
    shots.shots[1].text.ko.headline = ['대화를 읽고 한', '줄로 답해요'];
    shots.shots[2].text.en.headline = ['a', 'b', 'c'];
    expect(shotProblems(shots)).toEqual([
      expect.stringContaining('agents: en headline is one or two lines'),
      expect.stringContaining('conversation: ko accent "한 줄"'),
      expect.stringContaining('notification: en headline is one or two lines'),
    ]);
  });
});

describe('the frame', () => {
  const tokens = { field: '#B7C9CC', ink: '#0F1A1C', muted: '#3F5357', accent: '#B9FF66', bezel: '#101112', inter: '/f/Inter.woff2', pretendard: '/f/Pretendard.woff2' };

  it('marks the accent word and carries the headline and support line', async () => {
    const shots = await loadShots(real);
    const html = frameHtml({ shot: shots.shots[0], language: 'en', tokens, capturePath: '/raw/en-01-agents.png', canvas: shots.canvas });
    expect(html).toContain('<span class="line"><mark>Answer</mark> your agents.</span><span class="line">Away from your desk.</span>');
    expect(html).toContain('reply right there');
    expect(html).toContain('file:///raw/en-01-agents.png');
  });

  it('breaks the headline where shots.json breaks it, so an accent never sits on two lines', async () => {
    const shots = await loadShots(real);
    const html = frameHtml({ shot: shots.shots[1], language: 'ko', tokens, capturePath: '/raw/ko-02-conversation.png', canvas: shots.canvas });
    expect(html).toContain('<span class="line">대화를 읽고</span><span class="line"><mark>한 줄</mark>로 답해요</span>');
  });
});

describe('the PNG a shot is written as', () => {
  it('has its alpha channel removed and its pixels kept', () => {
    const flat = withoutAlpha(rgba());
    expect(pngInfo(flat)).toMatchObject({ width: 3, height: 2, bitDepth: 8, colorType: 2 });
    expect(withoutAlpha(flat)).toBe(flat);
    // Every pixel is as it was, whichever filter the rows were stored with: row 1 red, green, blue, row 2 three greys.
    const idat = [];
    for (let at = 8; at < flat.length;) { const length = flat.readUInt32BE(at); if (flat.toString('latin1', at + 4, at + 8) === 'IDAT') idat.push(flat.subarray(at + 8, at + 8 + length)); at += 12 + length; }
    expect([...inflateSync(Buffer.concat(idat))]).toEqual([0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 10, 20, 30, 40, 50, 60, 70, 80, 90]);
  });

  it('refuses a picture that is not fully opaque', () => {
    expect(() => withoutAlpha(rgba(200))).toThrow('not opaque');
  });
});

describe('the Play shot list as committed', () => {
  it('is valid and takes 4 or more of the iOS shots, none of them a notification', async () => {
    const [android, shots] = await Promise.all([loadAndroidShots(real), loadShots(real)]);
    expect(androidShotProblems(android, shots)).toEqual([]);
    expect(androidShotsOf(android, shots).length).toBeGreaterThanOrEqual(4);
    expect(androidShotsOf(android, shots).every((shot) => shot.capture === 'screen')).toBe(true);
    expect(Object.values(android.languages).map((one) => one.folder).sort()).toEqual(['en-US', 'ko-KR']);
  });

  it('plans one line per language and shot with the Play locale and the reply words in the steps', async () => {
    const [android, shots] = await Promise.all([loadAndroidShots(real), loadShots(real)]);
    const lines = planAndroid(android, shots, ['ko']).map((line) => line.split('\t'));
    expect(lines).toHaveLength(android.shots.length);
    expect(lines[1]).toEqual(['ko', '02-conversation', '12', 'ko-KR', expect.stringContaining('[data-phone-reply]::=네, 3건 모두 반영해 줘.')]);
    expect(lines.every((line) => line.length === 5 && !line[4].includes('{reply}'))).toBe(true);
  });

  it('names a wrong size, too few shots, a notification shot and a shot that is not there', async () => {
    const [android, shots] = await Promise.all([loadAndroidShots(real), loadShots(real)]);
    const broken = structuredClone(android);
    broken.canvas = { width: 1206, height: 2622 };
    broken.featureGraphic.width = 1025;
    broken.icon.size = 1024;
    broken.shots = ['agents', 'notification', 'nowhere'];
    expect(androidShotProblems(broken, shots)).toEqual(expect.arrayContaining([
      expect.stringContaining('1206x2622'),
      expect.stringContaining('feature graphic is 1025x500'),
      expect.stringContaining('icon is 1024'),
      expect.stringContaining('3 shots'),
      expect.stringContaining('notification: capture is "notification"'),
      expect.stringContaining('nowhere is not a shot'),
    ]));
  });

  it("wants the feature graphic's shot among the shots", async () => {
    const [android, shots] = await Promise.all([loadAndroidShots(real), loadShots(real)]);
    const broken = structuredClone(android);
    broken.shots = ['conversation', 'missions', 'pull-requests', 'looks'];
    expect(androidShotProblems(broken, shots)).toEqual([expect.stringContaining("feature graphic's shot agents is not one of the shots")]);
  });

  it("draws the feature graphic with the shot's words and the capture", async () => {
    const [android, shots] = await Promise.all([loadAndroidShots(real), loadShots(real)]);
    const tokens = { field: '#B7C9CC', ink: '#0F1A1C', muted: '#3F5357', accent: '#B9FF66', bezel: '#101112', inter: '/f/Inter.woff2', pretendard: '/f/Pretendard.woff2' };
    const html = featureHtml({ shot: shots.shots[0], language: 'en', tokens, capturePath: '/raw/en-01-agents.png', canvas: android.canvas, size: android.featureGraphic });
    expect(html).toContain('<mark>Answer</mark> your agents.');
    expect(html).toContain('width: 1024px; height: 500px');
    expect(html).toContain('file:///raw/en-01-agents.png');
  });
});

describe('the icon is 32-bit', () => {
  it('gets an opaque alpha channel and keeps its pixels', () => {
    const wide = withAlpha(withoutAlpha(rgba()));
    expect(pngInfo(wide)).toMatchObject({ width: 3, height: 2, colorType: 6 });
    expect(withAlpha(wide)).toBe(wide);
    expect(pngInfo(withoutAlpha(wide)).colorType).toBe(2);
  });
});
