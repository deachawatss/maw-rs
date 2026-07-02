import { fsList, fsRead } from "../../../../packages/wasm-sdk/assembly";
import { Host } from "@extism/as-pdk";

export function myAbort(message: string | null, fileName: string | null, lineNumber: u32, columnNumber: u32): void {}

function extractArgs(json: string): string[] { const marker = "\"args\":["; const start = json.indexOf(marker); if (start < 0) return []; let i = start + marker.length; const out = new Array<string>(); while (i < json.length && json.charAt(i) != "]") { if (json.charAt(i) == "\"") { const parsed = readJsonString(json, i); out.push(parsed.value); i = parsed.next; } else i++; } return out; }
class ParsedString { value: string; next: i32; constructor(value: string, next: i32) { this.value = value; this.next = next; } }
function readJsonString(s: string, start: i32): ParsedString { let out = ""; let i = start + 1; while (i < s.length) { const ch = s.charAt(i); if (ch == "\\") { i++; if (i >= s.length) break; const e = s.charAt(i); if (e == "n") out += "\n"; else if (e == "r") out += "\r"; else if (e == "t") out += "\t"; else out += e; } else if (ch == "\"") return new ParsedString(out, i + 1); else out += ch; i++; } return new ParsedString(out, i); }
function jsonStringField(json: string, key: string): string { const marker = "\"" + key + "\":"; const i = json.indexOf(marker); if (i < 0) return ""; let j = i + marker.length; while (j < json.length && json.charAt(j) != "\"") j++; if (j >= json.length) return ""; return readJsonString(json, j).value; }
function quote(value: string): string { let out = "\""; for (let i = 0; i < value.length; i++) { const code = value.charCodeAt(i); const ch = value.charAt(i); if (ch == "\\") out += "\\\\"; else if (ch == "\"") out += "\\\""; else if (ch == "\n") out += "\\n"; else if (ch == "\r") out += "\\r"; else if (ch == "\t") out += "\\t"; else if (code < 32 || code > 126) out += "\\u" + hex4(code); else out += ch; } return out + "\""; }
function hex4(code: i32): string { const digits = "0123456789abcdef"; return digits.charAt((code >> 12) & 15) + digits.charAt((code >> 8) & 15) + digits.charAt((code >> 4) & 15) + digits.charAt(code & 15); }
function finish(ok: bool, output: string | null, error: string | null): i32 { Host.outputString(resultJson(ok, output, error)); return 0; }
function resultJson(ok: bool, output: string | null, error: string | null): string { let json = ok ? "{\"ok\":true" : "{\"ok\":false"; if (ok) { if (output !== null) json += ",\"output\":" + quote(output); else json += ",\"output\":null"; } else { if (output !== null) json += ",\"output\":" + quote(output); if (error !== null) json += ",\"error\":" + quote(error); } return json + "}"; }

class Workspace { id: string; name: string; hubUrl: string; joinedAt: string; lastStatus: string; agents: string[]; constructor(id: string, name: string, hubUrl: string, joinedAt: string, lastStatus: string, agents: string[]) { this.id = id; this.name = name; this.hubUrl = hubUrl; this.joinedAt = joinedAt; this.lastStatus = lastStatus; this.agents = agents; } }
export function handle(): i32 {
  const args = extractArgs(Host.inputString());
  const sub = args.length > 0 ? args[0].toLowerCase() : "";
  if (sub != "" && sub != "ls" && sub != "list") return finish(false, null, "unsupported read-only workspace subcommand in WASM parity fixture");
  const list = fsList("{\"path\":\"/data/workspaces\",\"recursive\":false,\"includeDirs\":false}");
  const files = jsonArrayStrings(list, "entries");
  const rows = new Array<Workspace>();
  for (let i = 0; i < files.length; i++) {
    const body = readContent("/data/workspaces/" + files[i]);
    if (body == "") continue;
    rows.push(new Workspace(jsonStringField(body, "id"), jsonStringField(body, "name"), jsonStringField(body, "hubUrl"), jsonStringField(body, "joinedAt"), jsonStringField(body, "lastStatus"), jsonArrayStrings(body, "sharedAgents")));
  }
  if (rows.length == 0) return finish(true, "\u001b[90mNo workspaces configured.\u001b[0m\n\u001b[90m  maw workspace create <name>   Create a new workspace\u001b[0m\n\u001b[90m  maw workspace join <code>     Join with invite code\u001b[0m", null);
  let out = "\n\u001b[36;1mWorkspaces\u001b[0m  \u001b[90m" + rows.length.toString() + " joined\u001b[0m\n\n";
  for (let i = 0; i < rows.length; i++) {
    const ws = rows[i]; const dot = ws.lastStatus == "connected" ? "\u001b[32m●\u001b[0m" : "\u001b[31m●\u001b[0m"; const n = ws.agents.length; const label = n == 0 ? "\u001b[90mno agents shared\u001b[0m" : n.toString() + " agent" + (n != 1 ? "s" : "") + " shared";
    out += "  " + dot + "  \u001b[37;1m" + ws.name + "\u001b[0m  \u001b[90m(" + ws.id + ")\u001b[0m\n";
    out += "     \u001b[36mHub:\u001b[0m     " + ws.hubUrl + "\n";
    out += "     \u001b[36mAgents:\u001b[0m  " + label + "\n";
    if (n > 0) out += "     \u001b[90m         " + ws.agents.join(", ") + "\u001b[0m\n";
    out += "     \u001b[90mJoined:  " + ws.joinedAt + "\u001b[0m\n";
  }
  out += "";
  return finish(true, out, null);
}
function readContent(path: string): string { const out = fsRead("{\"path\":" + quote(path) + ",\"encoding\":\"utf8\"}"); if (out.indexOf("\"ok\":true") < 0) return ""; return jsonStringField(out, "content"); }
function jsonArrayStrings(json: string, key: string): string[] { const out = new Array<string>(); const marker = "\"" + key + "\":"; const i = json.indexOf(marker); if (i < 0) return out; let j = i + marker.length; while (j < json.length && json.charAt(j) != "[") j++; if (j >= json.length) return out; j++; while (j < json.length && json.charAt(j) != "]") { if (json.charAt(j) == "\"") { const parsed = readJsonString(json, j); out.push(parsed.value); j = parsed.next; } else j++; } return out; }
