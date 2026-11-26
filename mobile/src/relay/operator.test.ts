// Operator setup-code request and answers. Same answers as the Python and Rust editions.
import { test } from "node:test";
import assert from "node:assert/strict";
import { issueBody, issueSetupCode, parseIssue } from "./operator.ts";

test("the request body is passphrase, 20 uses and 7 days", () => {
  assert.deepEqual(JSON.parse(issueBody("pw")), { passphrase: "pw", max_uses: 20, days: 7 });
});

test("answers: 200 gives the code (normalized), 401, 429 and 503 say why, others are a status", () => {
  assert.equal(parseIssue(200, JSON.stringify({ code: "ab12-cd34" })), "AB12CD34");
  assert.throws(() => parseIssue(401, ""), /passphrase is wrong/);
  assert.throws(() => parseIssue(429, ""), /too many wrong tries/);
  assert.throws(() => parseIssue(503, ""), /secrets are not set/);
  assert.throws(() => parseIssue(500, ""), /status 500/);
  assert.throws(() => parseIssue(200, "not json"), /not JSON/);
  assert.throws(() => parseIssue(200, JSON.stringify({ code: "bad" })), /no valid code/);
});

test("nothing is sent over plain http, and the code goes to the function URL", async () => {
  const calls: string[] = [];
  const ok = async (url: string) => {
    calls.push(url);
    return { status: 200, text: async () => JSON.stringify({ code: "ABCDEFGH" }) };
  };
  await assert.rejects(issueSetupCode("http://x.supabase.co", "pw", ok), /https/);
  assert.equal(calls.length, 0);
  assert.equal(await issueSetupCode("https://x.supabase.co/", "pw", ok), "ABCDEFGH");
  assert.equal(calls[0], "https://x.supabase.co/functions/v1/issue-setup-code");
});

test("a network error gives one message without the passphrase", async () => {
  const down = async () => {
    throw new Error("socket pw");
  };
  await assert.rejects(issueSetupCode("https://x.supabase.co", "secret-pw", down), (error: Error) => {
    assert.equal(error.message, "the server could not be reached");
    assert.doesNotMatch(error.message, /secret-pw/);
    return true;
  });
});
