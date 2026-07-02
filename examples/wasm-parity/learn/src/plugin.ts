import { Host } from "@extism/as-pdk";
export function myAbort(message: string | null, fileName: string | null, lineNumber: u32, columnNumber: u32): void {}
export function handle(): i32 {
  const args = extractArgs(Host.inputString());
  const fast = contains(args, "--fast");
  const deep = contains(args, "--deep");
  if (fast && deep) return finish(false, null, "maw learn: --fast and --deep are mutually exclusive");
  const unknown = unknownFlags(args);
  if (unknown.length > 0) return finish(false, null, "maw learn: unknown flag(s) " + unknown.join(", ") + " (accepts --fast, --deep)");
  const repo = firstPositional(args);
  if (repo == "") return finish(false, null, "usage: maw learn <repo> [--fast|--deep]");
  const mode = fast ? "fast" : deep ? "deep" : "default";
  const agents = mode == "fast" ? 1 : mode == "deep" ? 5 : 3;
  const message = "learn: " + mode + " mode on \"" + repo + "\" — not yet implemented in core plugin; use Oracle skill /learn for full behavior.\n" +
    "  planned: " + agents.toString() + " parallel agent(s), write docs to ψ/learn/<owner>/<repo>/YYYY-MM-DD/HHMM_*.md\n" +
    "  track:   https://github.com/Soul-Brews-Studio/maw-js/issues/521";
  return finish(true, message, null);
}
function finish(ok: bool, output: string | null, error: string | null): i32 { Host.outputString(resultJson(ok, output, error)); return 0; }
function firstPositional(args: string[]): string { for (let i = 0; i < args.length; i++) if (!args[i].startsWith("--")) return args[i]; return ""; }
function contains(args: string[], needle: string): bool { for (let i = 0; i < args.length; i++) if (args[i] == needle) return true; return false; }
function unknownFlags(args: string[]): string[] { const out = new Array<string>(); for (let i = 0; i < args.length; i++) if (args[i].startsWith("--") && args[i] != "--fast" && args[i] != "--deep") out.push(args[i]); return out; }
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
function hex4(code: i32): string {
  const digits = "0123456789abcdef";
  return digits.charAt((code >> 12) & 15) + digits.charAt((code >> 8) & 15) + digits.charAt((code >> 4) & 15) + digits.charAt(code & 15);
}
