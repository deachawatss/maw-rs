import { Host } from "@extism/as-pdk";
import { fsRead, fsWrite } from "../../../../packages/wasm-sdk/assembly";

export function myAbort(message: string | null, fileName: string | null, lineNumber: u32, columnNumber: u32): void {}

const ACTIVE = "/config/profile-active";
const ALL = "/config/profiles/all.json";
const MINIMAL = "/config/profiles/minimal.json";

export function handle(): i32 {
  const args = extractArgs(Host.inputString()).filter((arg: string): bool => !arg.startsWith("--"));
  const sub = args.length > 0 ? args[0] : "";
  if (sub == "") return finish(true, help(), null);
  if (sub == "current" || sub == "active") return finish(true, activeProfile(), null);
  if (sub == "list" || sub == "ls") return finish(true, formatList(), null);
  if (sub == "show" || sub == "info") {
    if (args.length < 2 || args[1] == "") return finish(false, null, "usage: maw profile show <name>");
    const body = readProfile(args[1]);
    if (body == "") return finish(false, "", "profile \"" + args[1] + "\" not found");
    return finish(true, prettyProfile(body), null);
  }
  if (sub == "use" || sub == "set") {
    if (args.length < 2 || args[1] == "") return finish(false, null, "usage: maw profile use <name>");
    const body = readProfile(args[1]);
    if (body == "") return finish(false, "", "profile \"" + args[1] + "\" not found — see \"maw profile list\"");
    fsWrite("{\"path\":\"" + ACTIVE + "\",\"content\":" + quote(args[1] + "\n") + ",\"mode\":\"overwrite\",\"mkdirp\":true}");
    return finish(true, "active profile: \"" + profileName(body, args[1]) + "\"", null);
  }
  return finish(false, help(), "maw profile: unknown subcommand \"" + sub + "\" (expected list|use|show|current)");
}

function help(): string {
  return "usage: maw profile <list|use|show|current>\n" +
    "  list                 — list all profiles (active is marked with *)\n" +
    "  use     <name>       — set active profile (refuses unknown names)\n" +
    "  show    <name>       — print one profile's JSON\n" +
    "  current              — print active profile name\n" +
    "\n" +
    "storage:\n" +
    "  <CONFIG_DIR>/profiles/<name>.json   — one file per profile\n" +
    "  <CONFIG_DIR>/profile-active         — active profile pointer (text)\n" +
    "\n" +
    "note: Phase 1 of #640 — additive read + active-pointer only. Profile\n" +
    "      authoring is operator-driven (hand-edit JSON). Phase 2 wires this\n" +
    "      into the plugin loader.";
}

function activeProfile(): string {
  const raw = readContent(ACTIVE);
  const trimmed = trim(raw);
  if (trimmed == "") return "all";
  return isValidProfileName(trimmed) ? trimmed : "all";
}

function readProfile(name: string): string {
  if (!isValidProfileName(name)) return "";
  if (name == "all") return readContent(ALL);
  if (name == "minimal") return readContent(MINIMAL);
  return "";
}

function readContent(path: string): string {
  const out = fsRead("{\"path\":\"" + path + "\",\"encoding\":\"utf8\"}");
  if (out.indexOf("\"ok\":true") < 0) return "";
  const marker = "\"content\":";
  const i = out.indexOf(marker);
  if (i < 0) return "";
  let j = i + marker.length;
  while (j < out.length && out.charAt(j) != "\"") j++;
  if (j >= out.length) return "";
  return readJsonString(out, j).value;
}

function formatList(): string {
  const active = activeProfile();
  const all = readProfile("all");
  const minimal = readProfile("minimal");
  const rows = new Array<ProfileRow>();
  if (all != "") rows.push(new ProfileRow(profileName(all, "all"), pluginCount(all), tiersText(all), description(all)));
  if (minimal != "") rows.push(new ProfileRow(profileName(minimal, "minimal"), pluginCount(minimal), tiersText(minimal), description(minimal)));
  if (rows.length == 0) return "no profiles";
  let wName = 4;
  let wPlugins = 7;
  let wTiers = 5;
  for (let i = 0; i < rows.length; i++) {
    if (rows[i].name.length > wName) wName = rows[i].name.length;
    if (rows[i].plugins.length > wPlugins) wPlugins = rows[i].plugins.length;
    if (rows[i].tiers.length > wTiers) wTiers = rows[i].tiers.length;
  }
  const lines = new Array<string>();
  lines.push("   " + pad("name", wName) + "  " + pad("plugins", wPlugins) + "  " + pad("tiers", wTiers) + "  description");
  lines.push("   " + pad(repeat("-", wName), wName) + "  " + pad(repeat("-", wPlugins), wPlugins) + "  " + pad(repeat("-", wTiers), wTiers) + "  -----------");
  for (let i = 0; i < rows.length; i++) {
    const mark = rows[i].name == active ? "*" : " ";
    lines.push(" " + mark + " " + pad(rows[i].name, wName) + "  " + pad(rows[i].plugins, wPlugins) + "  " + pad(rows[i].tiers, wTiers) + "  " + rows[i].desc);
  }
  return lines.join("\n");
}

class ProfileRow { name: string; plugins: string; tiers: string; desc: string; constructor(name: string, plugins: string, tiers: string, desc: string) { this.name = name; this.plugins = plugins; this.tiers = tiers; this.desc = desc; } }

function profileName(json: string, fallback: string): string { const value = jsonStringField(json, "name"); return value == "" ? fallback : value; }
function description(json: string): string { return jsonStringField(json, "description"); }
function tiersText(json: string): string { const arr = jsonArrayStrings(json, "tiers"); return arr.length == 0 ? "-" : arr.join(","); }
function pluginCount(json: string): string { const arr = jsonArrayStrings(json, "plugins"); return arr.length == 0 ? "-" : arr.length.toString(); }

function prettyProfile(json: string): string {
  const name = profileName(json, "");
  const plugins = jsonArrayStrings(json, "plugins");
  const tiers = jsonArrayStrings(json, "tiers");
  const desc = description(json);
  let out = "{\n  \"name\": " + quote(name);
  if (plugins.length > 0) out += ",\n  \"plugins\": " + stringArrayJson(plugins);
  if (tiers.length > 0) out += ",\n  \"tiers\": " + stringArrayJson(tiers);
  if (desc != "") out += ",\n  \"description\": " + quote(desc);
  return out + "\n}";
}

function jsonStringField(json: string, key: string): string {
  const marker = "\"" + key + "\":";
  const i = json.indexOf(marker);
  if (i < 0) return "";
  let j = i + marker.length;
  while (j < json.length && json.charAt(j) != "\"") j++;
  if (j >= json.length) return "";
  return readJsonString(json, j).value;
}

function jsonArrayStrings(json: string, key: string): string[] {
  const out = new Array<string>();
  const marker = "\"" + key + "\":";
  const i = json.indexOf(marker);
  if (i < 0) return out;
  let j = i + marker.length;
  while (j < json.length && json.charAt(j) != "[") j++;
  if (j >= json.length) return out;
  j++;
  while (j < json.length && json.charAt(j) != "]") {
    if (json.charAt(j) == "\"") { const parsed = readJsonString(json, j); out.push(parsed.value); j = parsed.next; }
    else j++;
  }
  return out;
}

function extractArgs(json: string): string[] { const marker = "\"args\":["; const start = json.indexOf(marker); if (start < 0) return []; let i = start + marker.length; const out = new Array<string>(); while (i < json.length && json.charAt(i) != "]") { if (json.charAt(i) == "\"") { const parsed = readJsonString(json, i); out.push(parsed.value); i = parsed.next; } else i++; } return out; }
class ParsedString { value: string; next: i32; constructor(value: string, next: i32) { this.value = value; this.next = next; } }
function readJsonString(s: string, start: i32): ParsedString { let out = ""; let i = start + 1; while (i < s.length) { const ch = s.charAt(i); if (ch == "\\") { i++; if (i >= s.length) break; const e = s.charAt(i); if (e == "n") out += "\n"; else if (e == "r") out += "\r"; else if (e == "t") out += "\t"; else out += e; } else if (ch == "\"") return new ParsedString(out, i + 1); else out += ch; i++; } return new ParsedString(out, i); }
function finish(ok: bool, output: string | null, error: string | null): i32 { Host.outputString(resultJson(ok, output, error)); return 0; }
function resultJson(ok: bool, output: string | null, error: string | null): string { let json = ok ? "{\"ok\":true" : "{\"ok\":false"; if (ok) json += ",\"output\":\"\""; else if (output !== null) json += ",\"output\":" + quote(output); if (error !== null) json += ",\"error\":" + quote(error); return json + "}"; }
function quote(value: string): string { let out = "\""; for (let i = 0; i < value.length; i++) { const code = value.charCodeAt(i); const ch = value.charAt(i); if (ch == "\\") out += "\\\\"; else if (ch == "\"") out += "\\\""; else if (ch == "\n") out += "\\n"; else if (ch == "\r") out += "\\r"; else if (ch == "\t") out += "\\t"; else if (code < 32 || code > 126) out += "\\u" + hex4(code); else out += ch; } return out + "\""; }
function stringArrayJson(values: string[]): string { const parts = new Array<string>(); for (let i = 0; i < values.length; i++) parts.push(quote(values[i])); return "[" + parts.join(", ") + "]"; }
function isValidProfileName(name: string): bool { if (name.length == 0 || name.length > 64) return false; const first = name.charCodeAt(0); if (!isLowerNum(first)) return false; for (let i = 1; i < name.length; i++) { const c = name.charCodeAt(i); if (!isLowerNum(c) && c != 45 && c != 95) return false; } return true; }
function isLowerNum(c: i32): bool { return (c >= 48 && c <= 57) || (c >= 97 && c <= 122); }
function trim(value: string): string { let start = 0; let end = value.length; while (start < end && isSpace(value.charCodeAt(start))) start++; while (end > start && isSpace(value.charCodeAt(end - 1))) end--; return value.slice(start, end); }
function isSpace(c: i32): bool { return c == 32 || c == 9 || c == 10 || c == 13; }
function pad(value: string, width: i32): string { let out = value; while (out.length < width) out += " "; return out; }
function repeat(value: string, count: i32): string { let out = ""; for (let i = 0; i < count; i++) out += value; return out; }
function hex4(code: i32): string { const digits = "0123456789abcdef"; return digits.charAt((code >> 12) & 15) + digits.charAt((code >> 8) & 15) + digits.charAt((code >> 4) & 15) + digits.charAt(code & 15); }
