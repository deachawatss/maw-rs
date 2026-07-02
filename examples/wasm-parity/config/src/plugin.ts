import { Host } from "@extism/as-pdk";
import { saveConfig } from "../../../../packages/wasm-sdk/assembly";

export function myAbort(message: string | null, fileName: string | null, lineNumber: u32, columnNumber: u32): void {}

export function handle(): i32 {
  const args = extractArgs(Host.inputString());
  const sub = args.length > 0 ? args[0] : "show";
  const json = hasArg(args, "--json");

  if (sub == "set") {
    const key = args.length > 1 ? args[1] : "";
    const rawValue = args.length > 2 ? args[2] : "";
    if (key == "" || args.length < 3 || key.startsWith("-")) return finish(false, null, "usage: maw config set <key> <value>");
    if (isSecretKeyPath(key)) return finish(false, null, "maw config set: secret-like keys are host-gated and cannot be written from WASM");
    const value = parseConfigValue(rawValue);
    const request = "{\"key\":" + quote(key) + ",\"value\":" + value.json + ",\"patch\":" + objectPatchAtPath(key, value.json) + "}";
    const response = saveConfig(request);
    if (response.indexOf("\"ok\":true") < 0) return finish(false, null, hostError(response));
    const finalValue = readJsonValueAtKey(response, "finalValue");
    if (json) return finish(true, "{\n  \"key\": " + quote(key) + ",\n  \"value\": " + finalValue + "\n}", null);
    return finish(true, key + " = " + finalValue, null);
  }

  return finish(false, null, "usage: maw config <show|sources|explain <key>|set <key> <value>> [--json]");
}

class ConfigValue { json: string; constructor(json: string) { this.json = json; } }

function parseConfigValue(raw: string): ConfigValue {
  const trimmed = trim(raw);
  if (trimmed == "true" || trimmed == "false" || trimmed == "null") return new ConfigValue(trimmed);
  if (isNumberLiteral(trimmed)) return new ConfigValue(trimmed);
  if (((trimmed.startsWith("{") && trimmed.endsWith("}")) || (trimmed.startsWith("[") && trimmed.endsWith("]"))) && looksBalancedJson(trimmed)) return new ConfigValue(trimmed);
  return new ConfigValue(quote(raw));
}

function objectPatchAtPath(keyPath: string, valueJson: string): string {
  const parts = keyPath.split(".").filter((part: string): bool => part.length > 0);
  if (parts.length == 0) return "{}";
  let out = "";
  for (let i = 0; i < parts.length - 1; i++) out += "{" + quote(parts[i]) + ":";
  out += "{" + quote(parts[parts.length - 1]) + ":" + valueJson + "}";
  for (let i = 0; i < parts.length - 1; i++) out += "}";
  return out;
}

function isSecretKeyPath(key: string): bool {
  const lower = key.toLowerCase();
  return lower.includes("password") || lower.includes("passwd") || lower.includes("pwd") || lower.includes("credential") || lower.includes("private") || lower.includes("privatekey") || lower.includes("private_key") || lower.includes("passphrase") || lower.includes("cert") || lower.includes("pem") || lower.includes("secret") || lower.includes("token") || lower.includes("apikey") || lower.includes("api_key") || lower.includes("peerkey") || lower.includes("peer_key") || lower.includes("oauth") || lower.includes("auth_token") || lower.includes("auth-token") || lower.includes("authtoken") || lower.endsWith(".key") || lower.endsWith(".env") || lower == "key";
}

function hostError(response: string): string {
  const error = readJsonStringField(response, "error");
  return error == "" ? "maw.config.set failed" : error;
}

function readJsonValueAtKey(json: string, key: string): string {
  const marker = "\"" + key + "\":";
  const i = json.indexOf(marker);
  if (i < 0) return "null";
  let j = i + marker.length;
  while (j < json.length && isSpace(json.charCodeAt(j))) j++;
  return readJsonValue(json, j).value;
}

class ParsedString { value: string; next: i32; constructor(value: string, next: i32) { this.value = value; this.next = next; } }
function readJsonString(s: string, start: i32): ParsedString { let out = ""; let i = start + 1; while (i < s.length) { const ch = s.charAt(i); if (ch == "\\") { i++; if (i >= s.length) break; const e = s.charAt(i); if (e == "n") out += "\n"; else if (e == "r") out += "\r"; else if (e == "t") out += "\t"; else out += e; } else if (ch == "\"") return new ParsedString(out, i + 1); else out += ch; i++; } return new ParsedString(out, i); }
function readJsonStringField(json: string, key: string): string { const marker = "\"" + key + "\":"; const i = json.indexOf(marker); if (i < 0) return ""; let j = i + marker.length; while (j < json.length && json.charAt(j) != "\"") j++; if (j >= json.length) return ""; return readJsonString(json, j).value; }

class ParsedValue { value: string; next: i32; constructor(value: string, next: i32) { this.value = value; this.next = next; } }
function readJsonValue(s: string, start: i32): ParsedValue {
  if (start >= s.length) return new ParsedValue("null", start);
  const first = s.charAt(start);
  if (first == "\"") { const parsed = readJsonString(s, start); return new ParsedValue(quote(parsed.value), parsed.next); }
  let depth = 0;
  let i = start;
  while (i < s.length) {
    const ch = s.charAt(i);
    if (ch == "\"") { i = readJsonString(s, i).next; continue; }
    if (ch == "{" || ch == "[") depth++;
    else if (ch == "}" || ch == "]") { if (depth == 0) break; depth--; }
    else if ((ch == "," || ch == "}") && depth == 0) break;
    i++;
  }
  return new ParsedValue(trim(s.slice(start, i)), i);
}

function extractArgs(json: string): string[] { const marker = "\"args\":["; const start = json.indexOf(marker); if (start < 0) return []; let i = start + marker.length; const out = new Array<string>(); while (i < json.length && json.charAt(i) != "]") { if (json.charAt(i) == "\"") { const parsed = readJsonString(json, i); out.push(parsed.value); i = parsed.next; } else i++; } return out; }
function hasArg(args: string[], value: string): bool { for (let i = 0; i < args.length; i++) if (args[i] == value) return true; return false; }
function finish(ok: bool, output: string | null, error: string | null): i32 { Host.outputString(resultJson(ok, output, error)); return 0; }
function resultJson(ok: bool, output: string | null, error: string | null): string { let json = ok ? "{\"ok\":true" : "{\"ok\":false"; if (ok && output !== null) json += ",\"output\":" + quote(output); else if (ok) json += ",\"output\":\"\""; else if (output !== null) json += ",\"output\":" + quote(output); if (error !== null) json += ",\"error\":" + quote(error); return json + "}"; }
function quote(value: string): string { let out = "\""; for (let i = 0; i < value.length; i++) { const code = value.charCodeAt(i); const ch = value.charAt(i); if (ch == "\\") out += "\\\\"; else if (ch == "\"") out += "\\\""; else if (ch == "\n") out += "\\n"; else if (ch == "\r") out += "\\r"; else if (ch == "\t") out += "\\t"; else if (code < 32 || code > 126) out += "\\u" + hex4(code); else out += ch; } return out + "\""; }
function trim(value: string): string { let start = 0; let end = value.length; while (start < end && isSpace(value.charCodeAt(start))) start++; while (end > start && isSpace(value.charCodeAt(end - 1))) end--; return value.slice(start, end); }
function isSpace(c: i32): bool { return c == 32 || c == 9 || c == 10 || c == 13; }
function isDigit(c: i32): bool { return c >= 48 && c <= 57; }
function isNumberLiteral(value: string): bool { if (value.length == 0) return false; let i = value.charAt(0) == "-" ? 1 : 0; if (i >= value.length) return false; let digits = 0; while (i < value.length && isDigit(value.charCodeAt(i))) { i++; digits++; } if (digits == 0) return false; if (i < value.length && value.charAt(i) == ".") { i++; let frac = 0; while (i < value.length && isDigit(value.charCodeAt(i))) { i++; frac++; } if (frac == 0) return false; } return i == value.length; }
function looksBalancedJson(value: string): bool { let depth = 0; for (let i = 0; i < value.length; i++) { const ch = value.charAt(i); if (ch == "\"") { i = readJsonString(value, i).next - 1; continue; } if (ch == "{" || ch == "[") depth++; else if (ch == "}" || ch == "]") depth--; if (depth < 0) return false; } return depth == 0; }
function hex4(code: i32): string { const digits = "0123456789abcdef"; return digits.charAt((code >> 12) & 15) + digits.charAt((code >> 8) & 15) + digits.charAt((code >> 4) & 15) + digits.charAt(code & 15); }
