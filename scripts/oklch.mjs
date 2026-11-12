// OKLCH to sRGB hex, for the palette steps in design/tokens.json.
//
// A palette step is written as [lightness 0-100, chroma, hue] so the steps are
// spaced by perceived lightness; the shell draws the hex this returns. A color
// outside sRGB keeps its lightness and hue and loses chroma until it fits,
// the way a browser maps an out-of-gamut oklch() for display.

const cube = x => x * x * x;

function linear(l, c, h) {
  const a = c * Math.cos((h * Math.PI) / 180);
  const b = c * Math.sin((h * Math.PI) / 180);
  const L = l / 100;
  const lc = cube(L + 0.3963377774 * a + 0.2158037573 * b);
  const mc = cube(L - 0.1055613458 * a - 0.0638541728 * b);
  const sc = cube(L - 0.0894841775 * a - 1.291485548 * b);
  return [
    4.0767416621 * lc - 3.3077115913 * mc + 0.2309699292 * sc,
    -1.2684380046 * lc + 2.6097574011 * mc - 0.3413193965 * sc,
    -0.0041960863 * lc - 0.7034186147 * mc + 1.707614701 * sc,
  ];
}

const inGamut = rgb => rgb.every(x => x >= -1e-4 && x <= 1 + 1e-4);
const encode = x => (x <= 0.0031308 ? 12.92 * x : 1.055 * x ** (1 / 2.4) - 0.055);

/** The hex an [L, C, H] step draws. */
export function oklchHex([l, c, h]) {
  if (!(l >= 0 && l <= 100) || !(c >= 0) || !Number.isFinite(h)) throw new Error(`Not an OKLCH step: ${JSON.stringify([l, c, h])}`);
  let rgb = linear(l, c, h);
  if (!inGamut(rgb)) {
    let lo = 0, hi = c;
    for (let i = 0; i < 24; i++) {
      const mid = (lo + hi) / 2;
      if (inGamut(linear(l, mid, h))) lo = mid; else hi = mid;
    }
    rgb = linear(l, lo, h);
  }
  return `#${rgb.map(x => Math.round(Math.min(1, Math.max(0, encode(Math.min(1, Math.max(0, x))))) * 255).toString(16).padStart(2, '0')).join('')}`;
}
