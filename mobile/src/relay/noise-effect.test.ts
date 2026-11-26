// Noise overlay patterns: scanline count and spacing, speck count limits, and positions that stay on screen and repeat.
import { test } from "node:test";
import assert from "node:assert/strict";
import { DOT_SIZE, LINE_HEIGHT, MAX_DOTS, SCANLINES, dotCount, dotPositions, scanlineYs } from "../noise-effect.ts";

test("there are 300 scanlines, evenly spaced over the height, the first at the top, all on screen", () => {
  const ys = scanlineYs(844);
  assert.equal(ys.length, SCANLINES);
  assert.equal(SCANLINES, 300);
  assert.equal(ys[0], 0);
  assert.equal(ys[1], Math.floor(844 / 300));
  for (let index = 1; index < ys.length; index++) {
    assert.ok(ys[index] > ys[index - 1], "strictly increasing");
    assert.ok(ys[index] + LINE_HEIGHT <= 844, "inside the screen");
    assert.ok(Math.abs(ys[index] - index * (844 / 300)) < 1, "evenly spaced");
  }
});

test("the scanline spacing follows the height: a taller screen spreads the same lines further apart", () => {
  const short = scanlineYs(600);
  const tall = scanlineYs(1200);
  assert.equal(short.length, tall.length);
  assert.equal(tall[299], Math.floor(299 * 4));
  assert.equal(short[299], Math.floor(299 * 2));
});

test("the speck count follows the screen size and is capped", () => {
  assert.equal(dotCount(0, 0), 0);
  assert.equal(dotCount(390, 844), Math.round((390 * 844) / 3000));
  assert.equal(dotCount(4000, 4000), MAX_DOTS);
  assert.equal(dotCount(-5, 100), 0);
});

test("every speck is inside the screen, and the pattern is the same on each call", () => {
  const first = dotPositions(390, 844);
  assert.equal(first.length, dotCount(390, 844));
  for (const dot of first) {
    assert.ok(dot.x >= 0 && dot.x + DOT_SIZE <= 390);
    assert.ok(dot.y >= 0 && dot.y + DOT_SIZE <= 844);
  }
  assert.deepEqual(dotPositions(390, 844), first);
});
