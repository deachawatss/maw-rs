import { createHash } from 'node:crypto';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';

const [wasmPath, metadataPath] = process.argv.slice(2);
if (!wasmPath || !metadataPath) {
  throw new Error('usage: write-fixture-metadata <plugin.wasm> <metadata.json>');
}
const bytes = readFileSync(wasmPath);
const sha256 = createHash('sha256').update(bytes).digest('hex');
const existing = existsSync(metadataPath) ? JSON.parse(readFileSync(metadataPath, 'utf8')) : {};
const metadata = {
  assemblyscript: '0.27.31',
  export: 'handle',
  extismAsPdk: '1.0.0',
  ...(existing.mawJsCommit ? { mawJsCommit: existing.mawJsCommit } : {}),
  ...(existing.mawJsVersion ? { mawJsVersion: existing.mawJsVersion } : {}),
  wasmBytes: bytes.length,
  wasmSha256: `sha256:${sha256}`
};
writeFileSync(metadataPath, `${JSON.stringify(metadata, null, 2)}\n`);
