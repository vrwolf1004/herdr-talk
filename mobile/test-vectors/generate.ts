// Writes test-vectors/relay.json from the JavaScript side. Run: node test-vectors/generate.ts
import { writeFileSync } from "node:fs";
import { computeVectors, INPUTS } from "../src/relay/vectors.ts";

const out = { inputs: INPUTS, outputs: computeVectors() };
writeFileSync(new URL("./relay.json", import.meta.url), JSON.stringify(out, null, 2) + "\n");
console.log("wrote test-vectors/relay.json");
