// Node test (node --test): the issue request shape, the code alphabet, and the passphrase comparison.
import assert from "node:assert/strict";
import { test } from "node:test";
import { checkIssueBody, codeFromBytes, sameText, CODE_ALPHABET } from "./checks.ts";

test("the body needs a passphrase, max_uses 1 to 1000 and days 1 to 30", () => {
  assert.deepEqual(checkIssueBody({ passphrase: "p", max_uses: 20, days: 7 }), { passphrase: "p", max_uses: 20, days: 7 });
  assert.equal(checkIssueBody({ passphrase: "", max_uses: 20, days: 7 }), null);
  assert.equal(checkIssueBody({ passphrase: "p", max_uses: 0, days: 7 }), null);
  assert.equal(checkIssueBody({ passphrase: "p", max_uses: 1001, days: 7 }), null);
  assert.equal(checkIssueBody({ passphrase: "p", max_uses: 20, days: 31 }), null);
  assert.equal(checkIssueBody({ passphrase: "p", max_uses: 2.5, days: 7 }), null);
  assert.equal(checkIssueBody("nope"), null);
});

test("a generated code is 8 characters of A-Z and 0-9", () => {
  const code = codeFromBytes(new Uint8Array([0, 35, 36, 255, 1, 2, 3, 4]));
  assert.equal(code.length, 8);
  for (const ch of code) assert.ok(CODE_ALPHABET.includes(ch), ch);
});

test("the passphrase comparison is exact", () => {
  assert.equal(sameText("secret", "secret"), true);
  assert.equal(sameText("secret", "secreT"), false);
  assert.equal(sameText("secret", "secrets"), false);
});
