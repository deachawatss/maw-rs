import { fsList, fsRead, fsRemove } from "../../../../packages/wasm-sdk/assembly";
import { Host } from "@extism/as-pdk";

export function myAbort(message: string | null, fileName: string | null, lineNumber: u32, columnNumber: u32): void {}

class ParsedString { value: string; next: i32; constructor(value: string, next: i32) { this.value = value; this.next = next; } }
class Row { path: string; repo: string; mainRepo: string; mainPath: string; name: string; branch: string; classification: string; reason: string; removed: bool; error: string;
  constructor(path: string, repo: string, mainRepo: string, mainPath: string, name: string, branch: string, classification: string, reason: string) { this.path = path; this.repo = repo; this.mainRepo = mainRepo; this.mainPath = mainPath; this.name = name; this.branch = branch; this.classification = classification; this.reason = reason; this.removed = false; this.error = ""; }
}

export function handle(): i32 {
  const args = extractArgs(Host.inputString());
  const yes = has(args, "--yes") || has(args, "-y");
  const json = has(args, "--json");
  if (has(args, "--zombie-agents") || has(args, "--zombies") || has(args, "--prune-stale")) return finish(false, null, "unsupported destructive cleanup mode in WASM parity fixture; use maw-js for tmux/oracle registry cleanup");
  if (!has(args, "--worktrees")) return finish(true, "\u001b[36mmaw cleanup\u001b[0m — Cleanup utilities\n\n  maw cleanup --zombie-agents [--yes]              Find and kill orphan zombie panes\n  maw cleanup --zombies [--yes]                    Alias for --zombie-agents\n  maw cleanup --worktrees [--yes] [--json] [--repo <name>] [--scope .]  Survey and safe-remove orphan agent worktrees\n  maw cleanup --prune-stale [--yes|--ask|--dry-run]  Prune dead oracles.json entries\n\n\u001b[90mWithout --yes, only lists candidates without modifying anything.\u001b[0m", null);

  const rows = loadRows();
  if (yes) {
    for (let i = 0; i < rows.length; i++) {
      if (rows[i].classification != "CLEAN") continue;
      const response = fsRemove("{\"path\":" + quote(rows[i].path) + ",\"recursive\":true}");
      if (response.indexOf("\"ok\":true") >= 0) rows[i].removed = true;
      else rows[i].error = hostError(response);
    }
  }
  if (json) return finish(true, "{\n  \"ok\": true,\n  \"worktrees\": " + rowsJson(rows) + "\n}", null);
  let out = "\u001b[36mmaw cleanup --worktrees\u001b[0m — orphan worktree survey";
  if (rows.length == 0) out += "\n  \u001b[32m✓\u001b[0m no agent worktrees found";
  for (let i = 0; i < rows.length; i++) {
    const row = rows[i];
    out += "\n  " + pad(row.classification, 5) + " " + pad(row.name, 24) + " " + pad(row.branch == "" ? "-" : row.branch, 24) + " " + row.reason;
    out += "\n        \u001b[90m" + row.path + "\u001b[0m";
  }
  if (!yes) out += "\n\nDry-run only. Run with \u001b[36m--yes\u001b[0m to remove CLEAN worktrees.";
  return finish(true, out, null);
}

function loadRows(): Row[] {
  const list = fsList("{\"path\":\"/data/worktrees\",\"recursive\":false,\"includeDirs\":false}");
  const files = jsonArrayStrings(list, "entries");
  const rows = new Array<Row>();
  for (let i = 0; i < files.length; i++) {
    const body = readContent("/data/worktrees/" + files[i]);
    if (body == "") continue;
    rows.push(new Row(jsonStringField(body, "path"), jsonStringField(body, "repo"), jsonStringField(body, "mainRepo"), jsonStringField(body, "mainPath"), jsonStringField(body, "name"), jsonStringField(body, "branch"), jsonStringField(body, "classification"), jsonStringField(body, "reason")));
  }
  return rows;
}
function readContent(path: string): string { const out = fsRead("{\"path\":" + quote(path) + ",\"encoding\":\"utf8\"}"); if (out.indexOf("\"ok\":true") < 0) return ""; return jsonStringField(out, "content"); }
function hostError(response: string): string { const error = jsonStringField(response, "error"); return error == "" ? "maw.fs.remove failed" : error; }
function rowsJson(rows: Row[]): string { let out = "["; for (let i = 0; i < rows.length; i++) { if (i > 0) out += ","; const row = rows[i]; out += "\n    {\n      \"path\": " + quote(row.path) + ",\n      \"repo\": " + quote(row.repo) + ",\n      \"mainRepo\": " + quote(row.mainRepo) + ",\n      \"mainPath\": " + quote(row.mainPath) + ",\n      \"name\": " + quote(row.name) + ",\n      \"branch\": " + quote(row.branch) + ",\n      \"classification\": " + quote(row.classification) + ",\n      \"reason\": " + quote(row.reason); if (row.removed) out += ",\n      \"removed\": true"; if (row.error != "") out += ",\n      \"error\": " + quote(row.error); out += "\n    }"; } if (rows.length > 0) out += "\n  "; return out + "]"; }
function extractArgs(json: string): string[] { const marker = "\"args\":["; const start = json.indexOf(marker); if (start < 0) return []; let i = start + marker.length; const out = new Array<string>(); while (i < json.length && json.charAt(i) != "]") { if (json.charAt(i) == "\"") { const parsed = readJsonString(json, i); out.push(parsed.value); i = parsed.next; } else i++; } return out; }
function has(args: string[], value: string): bool { for (let i = 0; i < args.length; i++) if (args[i] == value) return true; return false; }
function jsonArrayStrings(json: string, key: string): string[] { const out = new Array<string>(); const marker = "\"" + key + "\":"; const i = json.indexOf(marker); if (i < 0) return out; let j = i + marker.length; while (j < json.length && json.charAt(j) != "[") j++; if (j >= json.length) return out; j++; while (j < json.length && json.charAt(j) != "]") { if (json.charAt(j) == "\"") { const parsed = readJsonString(json, j); out.push(parsed.value); j = parsed.next; } else j++; } return out; }
function jsonStringField(json: string, key: string): string { const marker = "\"" + key + "\":"; const i = json.indexOf(marker); if (i < 0) return ""; let j = i + marker.length; while (j < json.length && json.charAt(j) != "\"") j++; if (j >= json.length) return ""; return readJsonString(json, j).value; }
function readJsonString(s: string, start: i32): ParsedString { let out = ""; let i = start + 1; while (i < s.length) { const ch = s.charAt(i); if (ch == "\\") { i++; if (i >= s.length) break; const e = s.charAt(i); if (e == "n") out += "\n"; else if (e == "r") out += "\r"; else if (e == "t") out += "\t"; else out += e; } else if (ch == "\"") return new ParsedString(out, i + 1); else out += ch; i++; } return new ParsedString(out, i); }
function finish(ok: bool, output: string | null, error: string | null): i32 { Host.outputString(resultJson(ok, output, error)); return 0; }
function resultJson(ok: bool, output: string | null, error: string | null): string { let json = ok ? "{\"ok\":true" : "{\"ok\":false"; if (ok) { if (output !== null) json += ",\"output\":" + quote(output); else json += ",\"output\":null"; } else { if (output !== null) json += ",\"output\":" + quote(output); if (error !== null) json += ",\"error\":" + quote(error); } return json + "}"; }
function quote(value: string): string { let out = "\""; for (let i = 0; i < value.length; i++) { const code = value.charCodeAt(i); const ch = value.charAt(i); if (ch == "\\") out += "\\\\"; else if (ch == "\"") out += "\\\""; else if (ch == "\n") out += "\\n"; else if (ch == "\r") out += "\\r"; else if (ch == "\t") out += "\\t"; else if (code < 32 || code > 126) out += "\\u" + hex4(code); else out += ch; } return out + "\""; }
function hex4(code: i32): string { const digits = "0123456789abcdef"; return digits.charAt((code >> 12) & 15) + digits.charAt((code >> 8) & 15) + digits.charAt((code >> 4) & 15) + digits.charAt(code & 15); }
function pad(value: string, width: i32): string { if (value.length >= width) return value; return value + " ".repeat(width - value.length); }
