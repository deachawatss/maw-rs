import { Host } from "@extism/as-pdk";

export function myAbort(message: string | null, fileName: string | null, lineNumber: u32, columnNumber: u32): void {}

const TRACK_URL = "https://github.com/Soul-Brews-Studio/maw-js/issues/523";
const ORACLE_SKILL = "Oracle skill /project";

export function handle(): i32 {
  const args = extractArgs(Host.inputString()).filter((arg: string): bool => !arg.startsWith("--"));
  const sub = args.length > 0 ? args[0] : "";
  if (sub == "") {
    const help = helpText();
    return finish(true, help, null);
  }
  if (sub == "learn") {
    if (args.length < 2 || args[1] == "") return finish(false, null, "usage: maw project learn <url>");
    return finish(true, stubLine("learn", "would clone \"" + args[1] + "\" and symlink into ψ/learn/<owner>/<repo>"), null);
  }
  if (sub == "incubate") {
    if (args.length < 2 || args[1] == "") return finish(false, null, "usage: maw project incubate <url>");
    return finish(true, stubLine("incubate", "would clone \"" + args[1] + "\" and symlink into ψ/incubate/<owner>/<repo>"), null);
  }
  if (sub == "find" || sub == "search") {
    if (args.length < 2 || args[1] == "") return finish(false, null, "usage: maw project find <query>");
    return finish(true, stubLine("find", "would search tracked repos for \"" + args[1] + "\" across ψ/learn and ψ/incubate"), null);
  }
  if (sub == "list") {
    return finish(true, stubLine("list", "would list all tracked repos from ψ/learn and ψ/incubate"), null);
  }
  const help = helpText();
  return finish(false, help, "maw project: unknown subcommand \"" + sub + "\" (expected learn|incubate|find|list)");
}

function stubLine(action: string, detail: string): string {
  return "project " + action + ": " + detail + " — not yet implemented in core plugin; use " + ORACLE_SKILL + " for full behavior.\n" +
    "  track: " + TRACK_URL;
}

function helpText(): string {
  return "usage: maw project <learn|incubate|find|list> [args...]\n" +
    "  learn    <url>   — clone repo for study (symlink in ψ/learn/)\n" +
    "  incubate <url>   — clone repo for development (symlink in ψ/incubate/)\n" +
    "  find     <query> — search tracked repos (alias: search)\n" +
    "  list             — list all tracked repos\n" +
    "\n" +
    "see " + ORACLE_SKILL + " for the full implementation (scaffold tracks " + TRACK_URL + ").";
}

function finish(ok: bool, output: string | null, error: string | null): i32 { Host.outputString(resultJson(ok, output, error)); return 0; }
function extractArgs(json: string): string[] { const marker = "\"args\":["; const start = json.indexOf(marker); if (start < 0) return []; let i = start + marker.length; const out = new Array<string>(); while (i < json.length && json.charAt(i) != "]") { if (json.charAt(i) == "\"") { const parsed = readJsonString(json, i); out.push(parsed.value); i = parsed.next; } else i++; } return out; }
class ParsedString { value: string; next: i32; constructor(value: string, next: i32) { this.value = value; this.next = next; } }
function readJsonString(s: string, start: i32): ParsedString { let out = ""; let i = start + 1; while (i < s.length) { const ch = s.charAt(i); if (ch == "\\") { i++; if (i >= s.length) break; const e = s.charAt(i); if (e == "n") out += "\n"; else if (e == "r") out += "\r"; else if (e == "t") out += "\t"; else out += e; } else if (ch == "\"") return new ParsedString(out, i + 1); else out += ch; i++; } return new ParsedString(out, i); }
function resultJson(ok: bool, output: string | null, error: string | null): string { let json = ok ? "{\"ok\":true" : "{\"ok\":false"; if (output !== null) json += ",\"output\":" + quote(output); if (error !== null) json += ",\"error\":" + quote(error); return json + "}"; }
function quote(value: string): string {
  let out = "\"";
  for (let i = 0; i < value.length; i++) {
    const code = value.charCodeAt(i);
    const ch = value.charAt(i);
    if (ch == "\\") out += "\\\\";
    else if (ch == "\"") out += "\\\"";
    else if (ch == "\n") out += "\\n";
    else if (ch == "\r") out += "\\r";
    else if (ch == "\t") out += "\\t";
    else if (code < 32 || code > 126) out += "\\u" + hex4(code);
    else out += ch;
  }
  return out + "\"";
}
function hex4(code: i32): string { const digits = "0123456789abcdef"; return digits.charAt((code >> 12) & 15) + digits.charAt((code >> 8) & 15) + digits.charAt((code >> 4) & 15) + digits.charAt(code & 15); }
