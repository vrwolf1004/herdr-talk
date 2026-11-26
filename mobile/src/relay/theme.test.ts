// Theme choice: a saved name comes back as the same choice; anything else is the default (Green).
import { test } from "node:test";
import assert from "node:assert/strict";
import { CHOICES, noiseText, parseChoice, parseNoise, palette } from "../theme.ts";

test("the noise setting is saved as on/off; nothing saved or anything else is off", () => {
  assert.equal(parseNoise(noiseText(true)), true);
  assert.equal(parseNoise(noiseText(false)), false);
  assert.equal(noiseText(false), "off");
  assert.equal(parseNoise(null), false);
  assert.equal(parseNoise("yes"), false);
});

test("each saved theme name comes back as itself; nothing saved or an unknown name is Green", () => {
  for (const choice of CHOICES) assert.equal(parseChoice(choice), choice);
  assert.equal(parseChoice(null), "Green");
  assert.equal(parseChoice("Blue"), "Green");
  assert.equal(parseChoice("amber"), "Green", "saved names are case-sensitive");
});

test("Green and Amber use the GUI's colors", () => {
  assert.equal(palette("Green").accent, "#46ff82");
  assert.equal(palette("Green").background, "#050a08");
  assert.equal(palette("Amber").text, "#ffb000");
});
