import assert from "node:assert/strict";
import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "..");
const identityPath = path.resolve(process.argv[2] ?? path.join(root, "packaging/windows/store-identity.json"));
const outputPath = path.join(root, "plugins/nodestorm/skills/nodestorm/scripts/store.json");
const identity = JSON.parse(await readFile(identityPath, "utf8"));

for (const field of ["identityName", "publisher", "productId", "executionAlias", "msixVersion"]) {
  assert.ok(identity[field] && !identity[field].startsWith("REPLACE_"), `Store identity field ${field} is not reserved`);
}
assert.equal(identity.msixVersion, "1.0.0.0");

await writeFile(outputPath, `${JSON.stringify({
  identityName: identity.identityName,
  publisher: identity.publisher,
  productId: identity.productId,
  executionAlias: identity.executionAlias,
  msixVersion: identity.msixVersion,
  version: "1.0.0",
}, null, 2)}\n`);
console.log(`Configured ${path.relative(root, outputPath)} for Store Product ID ${identity.productId}.`);
