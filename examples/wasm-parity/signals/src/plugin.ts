import { Host } from "@extism/as-pdk";
import { fsList, fsRead } from "../../../../packages/wasm-sdk/assembly";

export function myAbort(message: string | null, fileName: string | null, lineNumber: u32, columnNumber: u32): void {}

const FIXED_NOW_DAY = daysFromCivil(2026, 7, 2);

class SignalRow {
  kind: string;
  timestamp: string;
  bud: string;
  message: string;
  file: string;

  constructor(kind: string, timestamp: string, bud: string, message: string, file: string) {
    this.kind = kind;
    this.timestamp = timestamp;
    this.bud = bud;
    this.message = message;
    this.file = file;
  }
}

class ParsedString {
  value: string;
  next: i32;

  constructor(value: string, next: i32) {
    this.value = value;
    this.next = next;
  }
}

export function handle(): i32 {
  const args = extractArgs(Host.inputString());
  const days = intFlag(args, "--days", 7);
  const asJson = has(args, "--json");
  const root = stringFlag(args, "--root", "/data");
  const signals = scanSignals(root, days);

  if (asJson) return finish(true, signalsJson(signals), null);
  if (signals.length == 0) {
    return finish(true, "  \u001b[90mno signals in the last " + days.toString() + " days\u001b[0m", null);
  }

  let out = "\n  \u001b[36mBud signals\u001b[0m (last " + days.toString() + "d \u2014 " + signals.length.toString() + " total)\n\n";
  for (let i = 0; i < signals.length; i++) {
    out += formatSignal(signals[i]);
    if (i + 1 < signals.length) out += "\n";
  }
  out += "\n";
  return finish(true, out, null);
}

function scanSignals(root: string, days: i32): SignalRow[] {
  const dir = root + "/" + String.fromCharCode(0x03c8) + "/memory/signals";
  const list = fsList("{\"path\":" + quote(dir) + ",\"recursive\":false,\"includeDirs\":false}");
  const files = jsonArrayStrings(list, "entries");
  const cutoff = FIXED_NOW_DAY - days;
  const rows = new Array<SignalRow>();
  for (let i = 0; i < files.length; i++) {
    const file = files[i];
    if (!file.endsWith(".json")) continue;
    const body = readContent(dir + "/" + file);
    if (body == "") continue;
    const row = new SignalRow(
      jsonStringField(body, "kind"),
      jsonStringField(body, "timestamp"),
      jsonStringField(body, "bud"),
      jsonStringField(body, "message"),
      file,
    );
    if (row.kind == "" || row.timestamp == "" || row.bud == "" || row.message == "") continue;
    if (timestampDay(row.timestamp) >= cutoff) rows.push(row);
  }
  sortNewestFirst(rows);
  return rows;
}

function formatSignal(row: SignalRow): string {
  const color = row.kind == "alert" ? "\u001b[31m" : row.kind == "pattern" ? "\u001b[33m" : row.kind == "info" ? "\u001b[36m" : "\u001b[37m";
  return "  " + color + "[" + row.kind + "]\u001b[0m \u001b[90m" + row.timestamp.slice(0, 10) + "\u001b[0m " + row.bud + ": " + row.message;
}

function signalsJson(rows: SignalRow[]): string {
  if (rows.length == 0) return "[]";
  let out = "[";
  for (let i = 0; i < rows.length; i++) {
    const row = rows[i];
    out += "\n  {\n";
    out += "    \"kind\": " + quote(row.kind) + ",\n";
    out += "    \"timestamp\": " + quote(row.timestamp) + ",\n";
    out += "    \"bud\": " + quote(row.bud) + ",\n";
    out += "    \"message\": " + quote(row.message) + ",\n";
    out += "    \"file\": " + quote(row.file) + "\n";
    out += "  }";
    if (i + 1 < rows.length) out += ",";
  }
  return out + "\n]";
}

function readContent(path: string): string {
  const out = fsRead("{\"path\":" + quote(path) + ",\"encoding\":\"utf8\"}");
  if (out.indexOf("\"ok\":true") < 0) return "";
  return jsonStringField(out, "content");
}

function sortNewestFirst(rows: SignalRow[]): void {
  for (let i = 1; i < rows.length; i++) {
    const current = rows[i];
    let j = i - 1;
    while (j >= 0 && rows[j].timestamp < current.timestamp) {
      rows[j + 1] = rows[j];
      j--;
    }
    rows[j + 1] = current;
  }
}

function timestampDay(value: string): i32 {
  if (value.length < 10) return -2147483648;
  return daysFromCivil(toInt(value.slice(0, 4)), toInt(value.slice(5, 7)), toInt(value.slice(8, 10)));
}

function daysFromCivil(year: i32, month: i32, day: i32): i32 {
  let y = year;
  if (month <= 2) y -= 1;
  const era = y / 400;
  const yoe = y - era * 400;
  const m = month > 2 ? month - 3 : month + 9;
  const doy = (153 * m + 2) / 5 + day - 1;
  const doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
  return era * 146097 + doe - 719468;
}

function intFlag(args: string[], flag: string, fallback: i32): i32 {
  const prefix = flag + "=";
  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg == flag && i + 1 < args.length) return toInt(args[i + 1]);
    if (arg.startsWith(prefix)) return toInt(arg.slice(prefix.length));
  }
  return fallback;
}

function stringFlag(args: string[], flag: string, fallback: string): string {
  const prefix = flag + "=";
  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg == flag && i + 1 < args.length) return args[i + 1];
    if (arg.startsWith(prefix)) return arg.slice(prefix.length);
  }
  return fallback;
}

function has(args: string[], value: string): bool {
  for (let i = 0; i < args.length; i++) if (args[i] == value) return true;
  return false;
}

function toInt(value: string): i32 {
  let sign = 1;
  let i = 0;
  if (value.startsWith("-")) {
    sign = -1;
    i = 1;
  }
  let out = 0;
  for (; i < value.length; i++) {
    const c = value.charCodeAt(i);
    if (c < 48 || c > 57) break;
    out = out * 10 + (c - 48);
  }
  return out * sign;
}

function extractArgs(json: string): string[] {
  const marker = "\"args\":[";
  const start = json.indexOf(marker);
  if (start < 0) return [];
  let i = start + marker.length;
  const out = new Array<string>();
  while (i < json.length && json.charAt(i) != "]") {
    if (json.charAt(i) == "\"") {
      const parsed = readJsonString(json, i);
      out.push(parsed.value);
      i = parsed.next;
    } else {
      i++;
    }
  }
  return out;
}

function jsonArrayStrings(json: string, key: string): string[] {
  const out = new Array<string>();
  const marker = "\"" + key + "\":";
  const start = json.indexOf(marker);
  if (start < 0) return out;
  let i = start + marker.length;
  while (i < json.length && json.charAt(i) != "[") i++;
  if (i >= json.length) return out;
  i++;
  while (i < json.length && json.charAt(i) != "]") {
    if (json.charAt(i) == "\"") {
      const parsed = readJsonString(json, i);
      out.push(parsed.value);
      i = parsed.next;
    } else {
      i++;
    }
  }
  return out;
}

function jsonStringField(json: string, key: string): string {
  const marker = "\"" + key + "\":";
  const start = json.indexOf(marker);
  if (start < 0) return "";
  let i = start + marker.length;
  while (i < json.length && json.charAt(i) != "\"") i++;
  if (i >= json.length) return "";
  return readJsonString(json, i).value;
}

function readJsonString(s: string, start: i32): ParsedString {
  let out = "";
  let i = start + 1;
  while (i < s.length) {
    const ch = s.charAt(i);
    if (ch == "\\") {
      i++;
      if (i >= s.length) break;
      const e = s.charAt(i);
      if (e == "n") out += "\n";
      else if (e == "r") out += "\r";
      else if (e == "t") out += "\t";
      else if (e == "u" && i + 4 < s.length) {
        out += String.fromCharCode(hexValue(s.charCodeAt(i + 1)) * 4096 + hexValue(s.charCodeAt(i + 2)) * 256 + hexValue(s.charCodeAt(i + 3)) * 16 + hexValue(s.charCodeAt(i + 4)));
        i += 4;
      } else {
        out += e;
      }
    } else if (ch == "\"") {
      return new ParsedString(out, i + 1);
    } else {
      out += ch;
    }
    i++;
  }
  return new ParsedString(out, i);
}

function finish(ok: bool, output: string | null, error: string | null): i32 {
  Host.outputString(resultJson(ok, output, error));
  return 0;
}

function resultJson(ok: bool, output: string | null, error: string | null): string {
  let json = ok ? "{\"ok\":true" : "{\"ok\":false";
  if (ok) {
    if (output !== null) json += ",\"output\":" + quote(output);
  } else {
    if (output !== null) json += ",\"output\":" + quote(output);
    if (error !== null) json += ",\"error\":" + quote(error);
  }
  return json + "}";
}

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

function hexValue(code: i32): i32 {
  if (code >= 48 && code <= 57) return code - 48;
  if (code >= 65 && code <= 70) return code - 55;
  if (code >= 97 && code <= 102) return code - 87;
  return 0;
}

function hex4(code: i32): string {
  const digits = "0123456789abcdef";
  return digits.charAt((code >> 12) & 15) + digits.charAt((code >> 8) & 15) + digits.charAt((code >> 4) & 15) + digits.charAt(code & 15);
}
