import { Host } from "@extism/as-pdk";
import { sendKeys } from "../../../../packages/wasm-sdk/assembly";

export function myAbort(message: string | null, fileName: string | null, lineNumber: u32, columnNumber: u32): void {}

export function handle(): i32 {
  const args = extractArgs(Host.inputString());
  const parsed = parseSendArgs(args);
  if (parsed.error != "") return finish(false, null, parsed.error);

  const request = "{\"target\":" + quote(parsed.target) + ",\"keys\":[" + quote(parsed.text) + "],\"literal\":true,\"enter\":false}";
  const response = sendKeys(request);
  if (response.indexOf("\"ok\":true") < 0) return finish(false, null, hostError(response));
  return finish(true, "\u001b[32mtyped\u001b[0m → " + parsed.target + ": " + truncate(parsed.text), null);
}

class SendArgs {
  target: string;
  text: string;
  error: string;
  constructor(target: string, text: string, error: string) {
    this.target = target;
    this.text = text;
    this.error = error;
  }
}

function parseSendArgs(args: string[]): SendArgs {
  let targetIndex = -1;
  for (let i = 0; i < args.length; i++) {
    if (!args[i].startsWith("-")) { targetIndex = i; break; }
  }
  if (targetIndex < 0) return new SendArgs("", "", "usage: maw send <target> \"<text>\"");
  const target = args[targetIndex];
  const text = args.slice(targetIndex + 1).join(" ");
  if (text.length == 0) return new SendArgs("", "", "usage: maw send <target> \"<text>\" — text is required");
  return new SendArgs(target, text, "");
}

function hostError(response: string): string {
  const error = jsonStringField(response, "error");
  return error == "" ? "maw.tmux.send_keys failed" : error;
}

function truncate(s: string, n: i32 = 200): string {
  if (s.length <= n) return s;
  return s.slice(0, n) + "...";
}

function extractArgs(json: string): string[] { const marker = "\"args\":["; const start = json.indexOf(marker); if (start < 0) return []; let i = start + marker.length; const out = new Array<string>(); while (i < json.length && json.charAt(i) != "]") { if (json.charAt(i) == "\"") { const parsed = readJsonString(json, i); out.push(parsed.value); i = parsed.next; } else i++; } return out; }
class ParsedString { value: string; next: i32; constructor(value: string, next: i32) { this.value = value; this.next = next; } }
function readJsonString(s: string, start: i32): ParsedString { let out = ""; let i = start + 1; while (i < s.length) { const ch = s.charAt(i); if (ch == "\\") { i++; if (i >= s.length) break; const e = s.charAt(i); if (e == "n") out += "\n"; else if (e == "r") out += "\r"; else if (e == "t") out += "\t"; else out += e; } else if (ch == "\"") return new ParsedString(out, i + 1); else out += ch; i++; } return new ParsedString(out, i); }
function jsonStringField(json: string, key: string): string { const marker = "\"" + key + "\":"; const i = json.indexOf(marker); if (i < 0) return ""; let j = i + marker.length; while (j < json.length && json.charAt(j) != "\"") j++; if (j >= json.length) return ""; return readJsonString(json, j).value; }
function finish(ok: bool, output: string | null, error: string | null): i32 { Host.outputString(resultJson(ok, output, error)); return 0; }
function resultJson(ok: bool, output: string | null, error: string | null): string { let json = ok ? "{\"ok\":true" : "{\"ok\":false"; if (ok) { if (output !== null) json += ",\"output\":" + quote(output); else json += ",\"output\":\"\""; } else { if (output !== null) json += ",\"output\":" + quote(output); if (error !== null) json += ",\"error\":" + quote(error); } return json + "}"; }
function quote(value: string): string { let out = "\""; for (let i = 0; i < value.length; i++) { const code = value.charCodeAt(i); const ch = value.charAt(i); if (ch == "\\") out += "\\\\"; else if (ch == "\"") out += "\\\""; else if (ch == "\n") out += "\\n"; else if (ch == "\r") out += "\\r"; else if (ch == "\t") out += "\\t"; else if (code < 32 || code > 126) out += "\\u" + hex4(code); else out += ch; } return out + "\""; }
function hex4(code: i32): string { const digits = "0123456789abcdef"; return digits.charAt((code >> 12) & 15) + digits.charAt((code >> 8) & 15) + digits.charAt((code >> 4) & 15) + digits.charAt(code & 15); }
