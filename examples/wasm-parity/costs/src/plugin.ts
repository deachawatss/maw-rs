import { Host } from "@extism/as-pdk";
import { localserverRequest } from "../../../../packages/wasm-sdk/assembly";

export function myAbort(message: string | null, fileName: string | null, lineNumber: u32, columnNumber: u32): void {}

class AgentRow {
  name: string;
  totalTokens: i32;
  estimatedCents: i32;
  sessions: i32;
  turns: i32;
  lastActive: string;

  constructor(name: string, totalTokens: i32, estimatedCents: i32, sessions: i32, turns: i32, lastActive: string) {
    this.name = name;
    this.totalTokens = totalTokens;
    this.estimatedCents = estimatedCents;
    this.sessions = sessions;
    this.turns = turns;
    this.lastActive = lastActive;
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

class ParsedValue {
  value: string;
  next: i32;

  constructor(value: string, next: i32) {
    this.value = value;
    this.next = next;
  }
}

export function handle(): i32 {
  const args = extractArgs(Host.inputString());
  const daily = dailyDays(args);
  const asJson = has(args, "--json") || has(args, "-j");

  if (daily > 0) {
    const response = requestPath("/api/costs/daily?days=" + daily.toString());
    if (response.indexOf("\"ok\":true") < 0) return finish(false, null, hostError(response, "cannot reach maw server \u2014 is `maw serve` running?"));
    const body = jsonStringField(response, "body");
    if (asJson) return finish(true, body, null);
    return finish(false, null, "maw costs --daily human output is outside this WASM parity fixture; use --json");
  }

  const response = requestPath("/api/costs");
  if (response.indexOf("\"ok\":true") < 0) return finish(false, null, hostError(response, "cannot reach maw server \u2014 is `maw serve` running?"));
  const body = jsonStringField(response, "body");
  const error = jsonStringField(body, "error");
  if (error != "") return finish(false, null, error);
  const agents = parseAgents(body);
  if (agents.length == 0) return finish(true, "\u001b[90mno session data found\u001b[0m", null);
  return finish(true, formatCosts(agents, jsonNumberField(readJsonValueAtKey(body, "total"), "agents"), jsonNumberField(readJsonValueAtKey(body, "total"), "sessions"), jsonNumberField(readJsonValueAtKey(body, "total"), "tokens"), parseCents(jsonNumberTextField(readJsonValueAtKey(body, "total"), "cost"))), null);
}

function requestPath(path: string): string {
  return localserverRequest("{\"method\":\"GET\",\"path\":" + quote(path) + ",\"timeoutMs\":3000}");
}

function formatCosts(agents: AgentRow[], totalAgents: i32, totalSessions: i32, totalTokens: i32, totalCents: i32): string {
  let out = "\n\u001b[36mCOST TRACKING\u001b[0m  (" + totalAgents.toString() + " agents, " + totalSessions.toString() + " sessions)\n\n";
  const hdr = padEnd("Agent", 30) + "  " + padStart("Tokens", 14) + "  " + padStart("Est. Cost", 12) + "  " + padStart("Sessions", 10) + "  " + padStart("Turns", 8) + "  " + padStart("Last Active", 13);
  const sep = repeat(String.fromCharCode(0x2500), hdr.length);
  out += "  \u001b[90m" + hdr + "\u001b[0m\n";
  out += "  \u001b[90m" + sep + "\u001b[0m\n";
  for (let i = 0; i < agents.length; i++) {
    const agent = agents[i];
    const name = agent.name.length > 28 ? agent.name.slice(0, 27) + String.fromCharCode(0x2026) : agent.name;
    const color = agent.estimatedCents > 1000 ? "\u001b[31m" : agent.estimatedCents > 100 ? "\u001b[33m" : "\u001b[32m";
    const last = agent.lastActive == "" ? String.fromCharCode(0x2014) : agent.lastActive.slice(0, 10);
    out += "  " + padEnd(name, 30) + "  " + padStart(fmtNum(agent.totalTokens), 14) + "  " + color + padStart("$" + cents(agent.estimatedCents), 12) + "\u001b[0m  " + padStart(agent.sessions.toString(), 10) + "  " + padStart(agent.turns.toString(), 8) + "  " + padStart(last, 13) + "\n";
  }
  out += "  \u001b[90m" + sep + "\u001b[0m\n";
  const totalColor = totalCents > 5000 ? "\u001b[31m" : totalCents > 1000 ? "\u001b[33m" : "\u001b[32m";
  out += "  " + padEnd("TOTAL", 30) + "  " + padStart(fmtNum(totalTokens), 14) + "  " + totalColor + "$" + cents(totalCents) + "\u001b[0m  " + padStart(totalSessions.toString(), 10) + "\n";
  return out;
}

function parseAgents(json: string): AgentRow[] {
  const rows = new Array<AgentRow>();
  const objects = jsonObjectsInArray(json, "agents");
  for (let i = 0; i < objects.length; i++) {
    const obj = objects[i];
    rows.push(new AgentRow(
      jsonStringField(obj, "name"),
      jsonNumberField(obj, "totalTokens"),
      parseCents(jsonNumberTextField(obj, "estimatedCost")),
      jsonNumberField(obj, "sessions"),
      jsonNumberField(obj, "turns"),
      jsonStringField(obj, "lastActive"),
    ));
  }
  return rows;
}

function dailyDays(args: string[]): i32 {
  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg == "--daily") {
      if (i + 1 >= args.length || args[i + 1].startsWith("-")) return 7;
      return toInt(args[i + 1]);
    }
    if (arg.startsWith("--daily=")) return toInt(arg.slice(8));
  }
  return 0;
}

function jsonObjectsInArray(json: string, key: string): string[] {
  const out = new Array<string>();
  const marker = "\"" + key + "\":";
  let i = json.indexOf(marker);
  if (i < 0) return out;
  i += marker.length;
  while (i < json.length && json.charAt(i) != "[") i++;
  if (i >= json.length) return out;
  i++;
  while (i < json.length) {
    const ch = json.charAt(i);
    if (ch == "\"") {
      i = readJsonString(json, i).next;
      continue;
    }
    if (ch == "{") {
      const parsed = readJsonValue(json, i);
      out.push(parsed.value);
      i = parsed.next;
      continue;
    }
    if (ch == "]") break;
    i++;
  }
  return out;
}

function readJsonValueAtKey(json: string, key: string): string {
  const marker = "\"" + key + "\":";
  const start = json.indexOf(marker);
  if (start < 0) return "{}";
  let i = start + marker.length;
  while (i < json.length && isSpace(json.charCodeAt(i))) i++;
  return readJsonValue(json, i).value;
}

function readJsonValue(s: string, start: i32): ParsedValue {
  let depth = 0;
  let i = start;
  while (i < s.length) {
    const ch = s.charAt(i);
    if (ch == "\"") {
      i = readJsonString(s, i).next;
      continue;
    }
    if (ch == "{" || ch == "[") depth++;
    else if (ch == "}" || ch == "]") {
      depth--;
      if (depth == 0) {
        i++;
        break;
      }
    } else if (ch == "," && depth == 0) {
      break;
    }
    i++;
  }
  return new ParsedValue(s.slice(start, i), i);
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

function jsonNumberField(json: string, key: string): i32 {
  return toInt(jsonNumberTextField(json, key));
}

function jsonNumberTextField(json: string, key: string): string {
  const marker = "\"" + key + "\":";
  const start = json.indexOf(marker);
  if (start < 0) return "0";
  let i = start + marker.length;
  while (i < json.length && isSpace(json.charCodeAt(i))) i++;
  const begin = i;
  while (i < json.length) {
    const c = json.charCodeAt(i);
    if ((c < 48 || c > 57) && c != 45 && c != 46) break;
    i++;
  }
  return json.slice(begin, i);
}

function parseCents(value: string): i32 {
  const dot = value.indexOf(".");
  if (dot < 0) return toInt(value) * 100;
  const whole = toInt(value.slice(0, dot));
  let frac = value.slice(dot + 1);
  if (frac.length == 0) frac = "00";
  if (frac.length == 1) frac += "0";
  return whole * 100 + toInt(frac.slice(0, 2));
}

function cents(value: i32): string {
  const whole = value / 100;
  const frac = value % 100;
  return whole.toString() + "." + (frac < 10 ? "0" : "") + frac.toString();
}

function fmtNum(n: i32): string {
  if (n >= 1000000000) return oneDecimal(n, 1000000000) + "B";
  if (n >= 1000000) return oneDecimal(n, 1000000) + "M";
  if (n >= 1000) return oneDecimal(n, 1000) + "K";
  return n.toString();
}

function oneDecimal(n: i32, divisor: i32): string {
  const tenths = (n * 10) / divisor;
  return (tenths / 10).toString() + "." + (tenths % 10).toString();
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
      else out += e;
    } else if (ch == "\"") {
      return new ParsedString(out, i + 1);
    } else {
      out += ch;
    }
    i++;
  }
  return new ParsedString(out, i);
}

function hostError(response: string, fallback: string): string {
  const error = jsonStringField(response, "error");
  return error == "" ? fallback : error;
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

function padStart(value: string, width: i32): string {
  if (value.length >= width) return value;
  return repeat(" ", width - value.length) + value;
}

function padEnd(value: string, width: i32): string {
  if (value.length >= width) return value;
  return value + repeat(" ", width - value.length);
}

function repeat(value: string, count: i32): string {
  let out = "";
  for (let i = 0; i < count; i++) out += value;
  return out;
}

function isSpace(c: i32): bool {
  return c == 32 || c == 9 || c == 10 || c == 13;
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

function hex4(code: i32): string {
  const digits = "0123456789abcdef";
  return digits.charAt((code >> 12) & 15) + digits.charAt((code >> 8) & 15) + digits.charAt((code >> 4) & 15) + digits.charAt(code & 15);
}
