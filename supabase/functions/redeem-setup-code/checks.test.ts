// Node test (node --test): the code shape and the failure limit.
import assert from "node:assert/strict";
import { test } from "node:test";
import { normalizeSetupCode, tooManyFailures, FAILURE_LIMIT } from "./checks.ts";

test("the typed code is normalized: spaces and dashes dropped, letters uppercased", () => {
  assert.equal(normalizeSetupCode("k7p2-x9qm"), "K7P2X9QM");
  assert.equal(normalizeSetupCode(" 12345678 "), "12345678");
});

test("anything that is not 8 characters of A-Z or 0-9 is refused", () => {
  for (const bad of ["K7P2X9Q", "K7P2X9QMM", "K7P2X9Q!", "가나다라마바사아", "", undefined, 12345678]) {
    assert.equal(normalizeSetupCode(bad), null, String(bad));
  }
});

test("the failure limit refuses at the limit, not before", () => {
  assert.equal(tooManyFailures(FAILURE_LIMIT - 1), false);
  assert.equal(tooManyFailures(FAILURE_LIMIT), true);
});
