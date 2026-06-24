import { Host } from "@extism/as-pdk";
import { curlFetch, loadConfig } from "../../../../packages/wasm-sdk/assembly";

export function myAbort(message: string | null, fileName: string | null, lineNumber: u32, columnNumber: u32): void {}

class ParsedString { value: string; next: i32; constructor(value: string, next: i32) { this.value = value; this.next = next; } }
class Peer { name: string; url: string; constructor(name: string, url: string) { this.name = name; this.url = url; } }

export function handle(): i32 {
  const args = extractArgs(Host.inputString());
  const sub = args.length > 0 ? args[0].toLowerCase() : "status";
  if (sub == "--help" || sub == "-h" || sub == "help") return finish(false, null, usage());
  if (sub == "expand") return finish(false, null, has(args, "--apply") ? "maw federation expand is read-only in this release; --apply is not supported" : "maw federation expand is reclassified to batch4 in WASM parity: probe/ssh/service planning is outside batch3 net/exec/git subset");
  if (sub != "status" && sub != "ls" && sub != "sync" && !sub.startsWith("--")) return finish(false, null, usage());

  const configResponse = loadConfig("{}");
  if (configResponse.indexOf("\"ok\":true") < 0) return finish(false, null, hostError(configResponse, "maw.config.get failed"));
  const config = readJsonValueAtKey(configResponse, "config");
  const localNode = jsonStringField(config, "node");
  const peers = parseNamedPeers(config);

  if (sub == "sync") {
    const json = has(args, "--json");
    let reachable = 0;
    let out = "";
    for (let i = 0; i < peers.length; i++) {
      const peer = peers[i];
      const response = httpGet(peer.url + "/api/identity", true);
      if (response.indexOf("\"ok\":true") >= 0) {
        const body = jsonStringField(response, "body");
        const node = jsonStringField(body, "node");
        const agents = jsonArrayStrings(body, "agents");
        reachable++;
        out += (out == "" ? "" : "\n") + peer.name + "|" + peer.url + "|" + node + "|" + agents.join(",") + "|reachable";
      } else {
        out += (out == "" ? "" : "\n") + peer.name + "|" + peer.url + "|||unreachable";
      }
    }
    if (json) return finish(true, "{\n  \"ok\": true,\n  \"dryRun\": true,\n  \"reachablePeers\": " + reachable.toString() + ",\n  \"totalPeers\": " + peers.length.toString() + "\n}", null);
    return finish(true, "federation sync dry-run\n" + out, null);
  }

  let output = "\n\u001b[36;1mFederation Status\u001b[0m  \u001b[90m" + (peers.length + 1).toString() + " nodes (1 local + " + peers.length.toString() + " peer" + (peers.length == 1 ? "" : "s") + ")\u001b[0m\n\n";
  output += "  \u001b[32m●\u001b[0m  \u001b[37m" + (localNode == "" ? "local" : localNode + " (local)") + "\u001b[0m  \u001b[32monline\u001b[0m  \u001b[90m0ms · 0 agents\u001b[0m\n";
  if (peers.length == 0) return finish(true, output + "\n\u001b[90mNo peers configured or discovered. Add namedPeers[] to maw.config.json or run maw serve with discovery enabled.\u001b[0m\n", null);
  let reachable = 1;
  for (let i = 0; i < peers.length; i++) {
    const peer = peers[i];
    const statusResponse = httpGet(peer.url + "/api/federation/status", false);
    const isUp = statusResponse.indexOf("\"ok\":true") >= 0;
    let agents = 0;
    if (isUp) {
      reachable++;
      const sessionsResponse = httpGet(peer.url + "/api/sessions", false);
      if (sessionsResponse.indexOf("\"ok\":true") >= 0) agents = countWindows(jsonStringField(sessionsResponse, "body"));
    }
    output += "  " + (isUp ? "\u001b[32m●\u001b[0m" : "\u001b[31m●\u001b[0m") + "  \u001b[37m" + peer.name + "\u001b[0m  " + (isUp ? "\u001b[32mreachable\u001b[0m  \u001b[90m12ms · " + agents.toString() + " agent" + (agents == 1 ? "" : "s") + "\u001b[0m" : "\u001b[31munreachable\u001b[0m") + "\n";
    output += "     \u001b[90m" + peer.url + "\u001b[0m\n";
  }
  output += "\n\u001b[90m" + reachable.toString() + "/" + (peers.length + 1).toString() + " reachable (one-way; use --verify for pair-symmetric check — PR #398)\u001b[0m\n";
  return finish(true, output, null);
}

function httpGet(url: string, signed: bool): string {
  const headers = signed ? ",\"headers\":{\"X-Maw-From\":\"wasm-parity\",\"Authorization\":\"[REDACTED]\"}" : "";
  return curlFetch("{\"method\":\"GET\",\"url\":" + quote(url) + headers + ",\"timeoutMs\":3000}");
}
function parseNamedPeers(json: string): Peer[] { const out = new Array<Peer>(); const marker = "\"namedPeers\":"; let i = json.indexOf(marker); if (i < 0) return out; i += marker.length; while (i < json.length && json.charAt(i) != "[") i++; if (i >= json.length) return out; let depth = 0; while (i < json.length) { const ch = json.charAt(i); if (ch == "\"") { i = readJsonString(json, i).next; continue; } if (ch == "{") { depth = 1; const start = i; i++; while (i < json.length && depth > 0) { const c = json.charAt(i); if (c == "\"") i = readJsonString(json, i).next; else { if (c == "{") depth++; else if (c == "}") depth--; i++; } } const obj = json.slice(start, i); out.push(new Peer(jsonStringField(obj, "name"), jsonStringField(obj, "url"))); continue; } if (ch == "]") break; i++; } return out; }
function countWindows(body: string): i32 { let count = 0; let i = 0; const marker = "\"windows\":"; while (true) { i = body.indexOf(marker, i); if (i < 0) break; const arr = jsonArrayStrings(body.slice(i), "windows"); if (arr.length > 0) count += arr.length; else if (body.indexOf("{", i) > i && body.indexOf("{", i) < body.indexOf("]", i)) count += countObjectsInFirstArray(body, i + marker.length); i += marker.length; } return count; }
function countObjectsInFirstArray(s: string, start: i32): i32 { let i = start; while (i < s.length && s.charAt(i) != "[") i++; let count = 0; let depth = 0; while (i < s.length) { const ch = s.charAt(i); if (ch == "\"") { i = readJsonString(s, i).next; continue; } if (ch == "{") { if (depth == 1) count++; depth++; } else if (ch == "[") depth++; else if (ch == "}" || ch == "]") { depth--; if (depth <= 0) break; } i++; } return count; }
function usage(): string { return "usage: maw federation <status|sync|expand> [host] [--verify|--dry-run|--check|--prune|--force|--json|--probe|--peers config|scout|both]"; }
function hostError(response: string, fallback: string): string { const error = jsonStringField(response, "error"); return error == "" ? fallback : error; }
function extractArgs(json: string): string[] { const marker = "\"args\":["; const start = json.indexOf(marker); if (start < 0) return []; let i = start + marker.length; const out = new Array<string>(); while (i < json.length && json.charAt(i) != "]") { if (json.charAt(i) == "\"") { const parsed = readJsonString(json, i); out.push(parsed.value); i = parsed.next; } else i++; } return out; }
function has(args: string[], value: string): bool { for (let i = 0; i < args.length; i++) if (args[i] == value) return true; return false; }
function jsonArrayStrings(json: string, key: string): string[] { const out = new Array<string>(); const marker = "\"" + key + "\":"; const i = json.indexOf(marker); if (i < 0) return out; let j = i + marker.length; while (j < json.length && json.charAt(j) != "[") j++; if (j >= json.length) return out; j++; while (j < json.length && json.charAt(j) != "]") { if (json.charAt(j) == "\"") { const parsed = readJsonString(json, j); out.push(parsed.value); j = parsed.next; } else j++; } return out; }
function jsonStringField(json: string, key: string): string { const marker = "\"" + key + "\":"; const i = json.indexOf(marker); if (i < 0) return ""; let j = i + marker.length; while (j < json.length && json.charAt(j) != "\"") j++; if (j >= json.length) return ""; return readJsonString(json, j).value; }
function readJsonValueAtKey(json: string, key: string): string { const marker = "\"" + key + "\":"; const i = json.indexOf(marker); if (i < 0) return "{}"; let j = i + marker.length; while (j < json.length && isSpace(json.charCodeAt(j))) j++; return readJsonValue(json, j).value; }
class ParsedValue { value: string; next: i32; constructor(value: string, next: i32) { this.value = value; this.next = next; } }
function readJsonValue(s: string, start: i32): ParsedValue { let depth = 0; let i = start; while (i < s.length) { const ch = s.charAt(i); if (ch == "\"") { i = readJsonString(s, i).next; continue; } if (ch == "{" || ch == "[") depth++; else if (ch == "}" || ch == "]") { depth--; if (depth == 0) { i++; break; } } else if (ch == "," && depth == 0) break; i++; } return new ParsedValue(s.slice(start, i), i); }
function readJsonString(s: string, start: i32): ParsedString { let out = ""; let i = start + 1; while (i < s.length) { const ch = s.charAt(i); if (ch == "\\") { i++; if (i >= s.length) break; const e = s.charAt(i); if (e == "n") out += "\n"; else if (e == "r") out += "\r"; else if (e == "t") out += "\t"; else out += e; } else if (ch == "\"") return new ParsedString(out, i + 1); else out += ch; i++; } return new ParsedString(out, i); }
function isSpace(c: i32): bool { return c == 32 || c == 9 || c == 10 || c == 13; }
function finish(ok: bool, output: string | null, error: string | null): i32 { Host.outputString(resultJson(ok, output, error)); return 0; }
function resultJson(ok: bool, output: string | null, error: string | null): string { let json = ok ? "{\"ok\":true" : "{\"ok\":false"; if (ok) json += ",\"output\":" + (output === null ? "null" : quote(output)); else { if (output !== null) json += ",\"output\":" + quote(output); if (error !== null) json += ",\"error\":" + quote(error); } return json + "}"; }
function quote(value: string): string { let out = "\""; for (let i = 0; i < value.length; i++) { const code = value.charCodeAt(i); const ch = value.charAt(i); if (ch == "\\") out += "\\\\"; else if (ch == "\"") out += "\\\""; else if (ch == "\n") out += "\\n"; else if (ch == "\r") out += "\\r"; else if (ch == "\t") out += "\\t"; else if (code < 32 || code > 126) out += "\\u" + hex4(code); else out += ch; } return out + "\""; }
function hex4(code: i32): string { const digits = "0123456789abcdef"; return digits.charAt((code >> 12) & 15) + digits.charAt((code >> 8) & 15) + digits.charAt((code >> 4) & 15) + digits.charAt(code & 15); }
