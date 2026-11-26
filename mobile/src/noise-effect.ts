// Fixed patterns for the noise overlay (pure, so the tests run under node). Nothing here depends on time, so nothing moves.

/** Scanlines: this many thin lines, spaced evenly over the screen height. */
export const SCANLINES = 300;
export const LINE_HEIGHT = 1;

/** Specks: a small square, about one per 3000 square points, never more than 300. */
export const DOT_SIZE = 2;
export const MAX_DOTS = 300;
export const POINTS_PER_DOT = 3000;

/** The top y (points) of each scanline: evenly spaced over the height, the first at 0, all inside the screen. */
export function scanlineYs(height: number): number[] {
  const step = Math.max(0, height) / SCANLINES;
  return Array.from({ length: SCANLINES }, (_, index) => Math.floor(index * step));
}

/** How many specks a screen of this size gets. */
export function dotCount(width: number, height: number): number {
  return Math.min(MAX_DOTS, Math.round((Math.max(0, width) * Math.max(0, height)) / POINTS_PER_DOT));
}

/** Speck positions (x, y in points) inside the screen, from a fixed pseudo-random sequence (the same on each call). */
export function dotPositions(width: number, height: number): { x: number; y: number }[] {
  const count = dotCount(width, height);
  const out: { x: number; y: number }[] = [];
  let seed = 49297;
  const next = () => {
    seed = (seed * 9301 + 49297) % 233280;
    return seed / 233280;
  };
  for (let index = 0; index < count; index++) {
    out.push({ x: Math.floor(next() * Math.max(0, width - DOT_SIZE)), y: Math.floor(next() * Math.max(0, height - DOT_SIZE)) });
  }
  return out;
}
