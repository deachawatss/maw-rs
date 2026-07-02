#!/usr/bin/env bun
// maw-rs → bun bridge: invokes a maw-js TS plugin with proper InvokeContext
export {};
const entryPath = process.argv[2];
const pluginArgs = process.argv.slice(3);

if (!entryPath) {
  console.error("usage: bun-bridge.ts <entry.ts> [args...]");
  process.exit(1);
}

const mod = await import(Bun.pathToFileURL(entryPath).href);
const handler = mod.default;

if (typeof handler !== "function") {
  console.error(`plugin ${entryPath} has no default export function`);
  process.exit(1);
}

const ctx = {
  source: "cli",
  args: pluginArgs,
  matchedName: pluginArgs[0] || "",
  flags: {},
  writer: (...a: any[]) => console.log(...a),
  config: null,
};

try {
  const result = await handler(ctx);
  if (result?.output) console.log(result.output);
  if (result?.error) console.error(result.error);
  process.exit(result?.ok === false ? 1 : 0);
} catch (e: any) {
  console.error(e?.message || String(e));
  process.exit(1);
}
