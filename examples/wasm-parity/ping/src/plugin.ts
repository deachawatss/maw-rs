import { Host } from "@extism/as-pdk";
import { curlFetch, loadConfig } from "../../../../packages/wasm-sdk/assembly";

export function myAbort(message: string | null, fileName: string | null, lineNumber: u32, columnNumber: u32): void {}

class ParsedString { value: string; next: i32; constructor(value: string, next: i32) { this.value = value; this.next = next; } }
class Peer { name: string; url: string; constructor(name: string, url: string) { this.name = name; this.url = url; } }
export function handle(): i32 {
  const args = extractArgs(Host.inputString());
  const wanted = args.length > 0 ? args[0] : "";

  const configResponse = loadConfig("{}");
  if (configResponse.indexOf("\"ok\":true") < 0) return finish(false, null, hostError(configResponse, "maw.config.get failed"));
  const config = readJsonValueAtKey(configResponse, "config");
  const peers = parseTargets(config, wanted);

  if (wanted != "" && peers.length == 0) {
    const known = parseTargets(config, "");
    let names = "";
    for (let i = 0; i < known.length; i++) names += (i == 0 ? "" : ", ") + known[i].name;
    const output = "\u001b[33mknown\u001b[0m: " + (names == "" ? "(none)" : names);
    return finish(false, output, output);
  }
  if (peers.length == 0) return finish(true, "\u001b[90mno peers configured\u001b[0m", null);

  let output = "";
  for (let i = 0; i < peers.length; i++) {
    const peer = peers[i];
    const response = httpPing(peer.url + "/api/auth/status");
    const ok = response.indexOf("\"ok\":true") >= 0;
    const status = jsonNumberField(response, "status");
    const body = jsonStringField(response, "body");
    const data = readJsonValueAtKey(response, "data");
    const enabled = jsonBoolField(body, "enabled") || jsonBoolField(data, "enabled");
    const token = firstNonEmpty(jsonStringField(body, "tokenPreview"), jsonStringField(data, "tokenPreview"));
    let auth = ok ? (enabled ? "auth: ok" : "auth: off") : (status > 0 ? status.toString() : "unreachable");
    const line = ok
      ? "\u001b[32m✅\u001b[0m " + peer.name + " \u001b[90m(" + peer.url + ")\u001b[0m — 12ms, " + auth + (token != "" ? " (" + token + ")" : "")
      : "\u001b[31m❌\u001b[0m " + peer.name + " \u001b[90m(" + peer.url + ")\u001b[0m — 12ms, " + auth;
    output += (output == "" ? "" : "\n") + line;
  }
  return finish(true, output, null);
}

function httpPing(url: string): string {
  return curlFetch("{\"method\":\"GET\",\"url\":" + quote(url) + ",\"timeoutMs\":3000}");
}

function parseTargets(config: string, wanted: string): Peer[] {
  const peers = parseNamedPeers(config);
  const legacy = parseLegacyPeers(config, peers);
  const out = new Array<Peer>();
  if (wanted != "") {
    for (let i = 0; i < peers.length; i++) if (peers[i].name == wanted) out.push(peers[i]);
    for (let i = 0; i < legacy.length; i++) if (legacy[i].url.indexOf(wanted) >= 0) out.push(new Peer(wanted, legacy[i].url));
    return out;
  }
  for (let i = 0; i < peers.length; i++) out.push(peers[i]);
  for (let i = 0; i < legacy.length; i++) out.push(new Peer(legacy[i].url, legacy[i].url));
  return out;
}
function parseNamedPeers(json: string): Peer[] { const out = new Array<Peer>(); const marker = "\"namedPeers\":"; let i = json.indexOf(marker); if (i < 0) return out; i += marker.length; while (i < json.length && json.charAt(i) != "[") i++; if (i >= json.length) return out; while (i < json.length) { const ch = json.charAt(i); if (ch == "\"") { i = readJsonString(json, i).next; continue; } if (ch == "{") { let depth = 1; const start = i; i++; while (i < json.length && depth > 0) { const c = json.charAt(i); if (c == "\"") i = readJsonString(json, i).next; else { if (c == "{") depth++; else if (c == "}") depth--; i++; } } const obj = json.slice(start, i); out.push(new Peer(jsonStringField(obj, "name"), jsonStringField(obj, "url"))); continue; } if (ch == "]") break; i++; } return out; }
function parseLegacyPeers(json: string, named: Peer[]): Peer[] { const out = new Array<Peer>(); const urls = jsonArrayStrings(json, "peers"); for (let i = 0; i < urls.length; i++) { let duplicate = false; for (let j = 0; j < named.length; j++) if (named[j].url == urls[i]) duplicate = true; if (!duplicate) out.push(new Peer(urls[i], urls[i])); } return out; }
function firstNonEmpty(a: string, b: string): string { return a != "" ? a : b; }
function hostError(response: string, fallback: string): string { const error = jsonStringField(response, "error"); return error == "" ? fallback : error; }
function extractArgs(json: string): string[] { const marker = "\"args\":["; const start = json.indexOf(marker); if (start < 0) return []; let i = start + marker.length; const out = new Array<string>(); while (i < json.length && json.charAt(i) != "]") { if (json.charAt(i) == "\"") { const parsed = readJsonString(json, i); out.push(parsed.value); i = parsed.next; } else i++; } return out; }
function jsonArrayStrings(json: string, key: string): string[] { const out = new Array<string>(); const marker = "\"" + key + "\":"; const i = json.indexOf(marker); if (i < 0) return out; let j = i + marker.length; while (j < json.length && json.charAt(j) != "[") j++; if (j >= json.length) return out; j++; while (j < json.length && json.charAt(j) != "]") { if (json.charAt(j) == "\"") { const parsed = readJsonString(json, j); out.push(parsed.value); j = parsed.next; } else j++; } return out; }
function jsonStringField(json: string, key: string): string { const marker = "\"" + key + "\":"; const i = json.indexOf(marker); if (i < 0) return ""; let j = i + marker.length; while (j < json.length && json.charAt(j) != "\"") j++; if (j >= json.length) return ""; return readJsonString(json, j).value; }
function jsonBoolField(json: string, key: string): bool { const marker = "\"" + key + "\":"; const i = json.indexOf(marker); if (i < 0) return false; let j = i + marker.length; while (j < json.length && isSpace(json.charCodeAt(j))) j++; return json.slice(j, j + 4) == "true"; }
function jsonNumberField(json: string, key: string): i32 { const marker = "\"" + key + "\":"; const i = json.indexOf(marker); if (i < 0) return 0; let j = i + marker.length; while (j < json.length && isSpace(json.charCodeAt(j))) j++; let out = 0; while (j < json.length) { const c = json.charCodeAt(j); if (c < 48 || c > 57) break; out = out * 10 + (c - 48); j++; } return out; }
function readJsonValueAtKey(json: string, key: string): string { const marker = "\"" + key + "\":"; const i = json.indexOf(marker); if (i < 0) return "{}"; let j = i + marker.length; while (j < json.length && isSpace(json.charCodeAt(j))) j++; return readJsonValue(json, j).value; }
class ParsedValue { value: string; next: i32; constructor(value: string, next: i32) { this.value = value; this.next = next; } }
function readJsonValue(s: string, start: i32): ParsedValue { let depth = 0; let i = start; while (i < s.length) { const ch = s.charAt(i); if (ch == "\"") { i = readJsonString(s, i).next; continue; } if (ch == "{" || ch == "[") depth++; else if (ch == "}" || ch == "]") { depth--; if (depth == 0) { i++; break; } } else if (ch == "," && depth == 0) break; i++; } return new ParsedValue(s.slice(start, i), i); }
function readJsonString(s: string, start: i32): ParsedString { let out = ""; let i = start + 1; while (i < s.length) { const ch = s.charAt(i); if (ch == "\\") { i++; if (i >= s.length) break; const e = s.charAt(i); if (e == "n") out += "\n"; else if (e == "r") out += "\r"; else if (e == "t") out += "\t"; else out += e; } else if (ch == "\"") return new ParsedString(out, i + 1); else out += ch; i++; } return new ParsedString(out, i); }
function isSpace(c: i32): bool { return c == 32 || c == 9 || c == 10 || c == 13; }
function finish(ok: bool, output: string | null, error: string | null): i32 { Host.outputString(resultJson(ok, output, error)); return 0; }
function resultJson(ok: bool, output: string | null, error: string | null): string { let json = ok ? "{\"ok\":true" : "{\"ok\":false"; if (ok) json += ",\"output\":" + (output === null ? "null" : quote(output)); else { if (output !== null) json += ",\"output\":" + quote(output); if (error !== null) json += ",\"error\":" + quote(error); } return json + "}"; }
function quote(value: string): string { let out = "\""; for (let i = 0; i < value.length; i++) { const code = value.charCodeAt(i); const ch = value.charAt(i); if (ch == "\\") out += "\\\\"; else if (ch == "\"") out += "\\\""; else if (ch == "\n") out += "\\n"; else if (ch == "\r") out += "\\r"; else if (ch == "\t") out += "\\t"; else if (code < 32 || code > 126) out += "\\u" + hex4(code); else out += ch; } return out + "\""; }
function hex4(code: i32): string { const digits = "0123456789abcdef"; return digits.charAt((code >> 12) & 15) + digits.charAt((code >> 8) & 15) + digits.charAt((code >> 4) & 15) + digits.charAt(code & 15); }
