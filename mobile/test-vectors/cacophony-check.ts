// Checks the Noise IK first message against the standard Cacophony vector shipped with the Python edition.
import { readFileSync } from "node:fs";
import { publicKeyOf, writeMessage1 } from "../src/relay/noise.ts";
import { hexToBytes, bytesToHex } from "../src/relay/bytes.ts";

const v = JSON.parse(readFileSync(new URL("../../python/tests/noise_ik_vector.json", import.meta.url), "utf8"));
const m = v.messages[0];
const responderPublic = publicKeyOf(hexToBytes(v.resp_static));
const out = writeMessage1(hexToBytes(v.init_static), responderPublic, hexToBytes(m.payload), hexToBytes(v.init_ephemeral), hexToBytes(v.init_prologue));
console.log("match:", bytesToHex(out) === m.ciphertext);
console.log("got  ", bytesToHex(out).slice(0, 80));
console.log("want ", m.ciphertext.slice(0, 80));
