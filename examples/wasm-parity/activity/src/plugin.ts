import { Host } from "@extism/as-pdk";
import { capture } from "../../../../packages/wasm-sdk/assembly";

export function myAbort(message: string | null, fileName: string | null, lineNumber: u32, columnNumber: u32): void {}

const ACTIVITY_USAGE = "usage: maw activity <pane> [--watch] [--json] [--stuck-only] [--window=<dur>] [--samples=N] [--sampler=peek|follow] | maw activity --all [--watch] [--json] [--stuck-only] [--window=<dur>] [--samples=N] [--sampler=peek|follow]";

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
  const target = firstTarget(args);
  if (target == "" || target == "--help" || target == "-h" || has(args, "--all") || has(args, "--watch")) {
    return finish(false, null, ACTIVITY_USAGE);
  }
  if (stringFlag(args, "--sampler", "peek") != "peek") {
    return finish(false, null, "activity: --sampler must be peek or follow");
  }

  const samples = intFlag(args, "--samples", 3);
  if (samples < 2 || samples > 50) return finish(false, null, "activity: --samples must be an integer from 2 to 50");
  const windowSeconds = durationSeconds(stringFlag(args, "--window", "30s"));
  if (windowSeconds <= 0) return finish(false, null, "activity: invalid --window duration: " + stringFlag(args, "--window", ""));

  let diffSamples = 0;
  let previous = "";
  for (let i = 0; i < samples; i++) {
    const text = normalizeSnapshot(captureContent(target));
    if (i > 0 && text != previous) diffSamples += 2;
    previous = text;
  }
  if (diffSamples > samples) diffSamples = samples;

  const stuck = diffSamples == 0 && isStuckSnapshot(previous);
  const state = diffSamples > 0 ? "busy" : stuck ? "stuck" : "idle";
  const confidence = samples >= 3 ? "high" : samples == 2 ? "medium" : "low";
  const lastChange = diffSamples > 0 ? 0 : windowSeconds;

  if (has(args, "--stuck-only") && state != "stuck") return finish(true, "", null);
  if (has(args, "--json")) {
    return finish(true, "{\"pane\":\"" + target + "\",\"state\":\"" + state + "\",\"confidence\":\"" + confidence + "\",\"samples\":" + samples.toString() + ",\"diff_samples\":" + diffSamples.toString() + ",\"last_change_ago_seconds\":" + lastChange.toString() + ",\"sample_window_seconds\":" + windowSeconds.toString() + "}\n", null);
  }
  return finish(true, formatHuman(target, state, lastChange, diffSamples, samples) + "\n", null);
}

function captureContent(target: string): string {
  const response = capture("{\"target\":\"" + target + "\",\"lines\":80}");
  return jsonStringField(response, "content");
}

function formatHuman(pane: string, state: string, lastChange: i32, diffSamples: i32, samples: i32): string {
  const icon = state == "busy" ? String.fromCodePoint(0x1f7e2) : state == "stuck" ? String.fromCodePoint(0x1f534) : String.fromCodePoint(0x1f7e1);
  const upper = state.toUpperCase();
  const age = state == "busy"
    ? "last change " + formatDuration(lastChange) + " ago"
    : state == "stuck"
      ? "at prompt (no change in " + formatDuration(lastChange) + ")"
      : "quiet (no change in " + formatDuration(lastChange) + ")";
  return pane + ": " + icon + " " + upper + " (" + age + ", " + diffSamples.toString() + "/" + samples.toString() + " samples diff)";
}

function formatDuration(seconds: i32): string {
  if (seconds < 60) return seconds.toString() + "s";
  const minutes = (seconds + 30) / 60;
  if (minutes < 60) return minutes.toString() + "m";
  return ((minutes + 30) / 60).toString() + "h";
}

function normalizeSnapshot(input: string): string {
  return input.replace("\r", "\n").trim();
}

function isStuckSnapshot(input: string): bool {
  const normalized = normalizeSnapshot(input).toLowerCase();
  if (normalized.endsWith("type a message") || normalized.endsWith("send a message") || normalized.endsWith("ask codex") || normalized.endsWith("ask claude")) return true;
  const lines = normalized.split("\n");
  if (lines.length == 0) return false;
  const last = lines[lines.length - 1].trim();
  return last == ">" || last == "$" || last == "#";
}

function firstTarget(args: string[]): string {
  for (let i = 0; i < args.length; i++) {
    if (!args[i].startsWith("-")) return args[i];
  }
  return "";
}

function intFlag(args: string[], flag: string, fallback: i32): i32 {
  const prefix = flag + "=";
  for (let i = 0; i < args.length; i++) {
    if (args[i] == flag && i + 1 < args.length) return toInt(args[i + 1]);
    if (args[i].startsWith(prefix)) return toInt(args[i].slice(prefix.length));
  }
  return fallback;
}

function stringFlag(args: string[], flag: string, fallback: string): string {
  const prefix = flag + "=";
  for (let i = 0; i < args.length; i++) {
    if (args[i] == flag && i + 1 < args.length) return args[i + 1];
    if (args[i].startsWith(prefix)) return args[i].slice(prefix.length);
  }
  return fallback;
}

function durationSeconds(value: string): i32 {
  if (value.endsWith("ms")) return toInt(value.slice(0, value.length - 2)) / 1000;
  if (value.endsWith("s")) return toInt(value.slice(0, value.length - 1));
  if (value.endsWith("m")) return toInt(value.slice(0, value.length - 1)) * 60;
  if (value.endsWith("h")) return toInt(value.slice(0, value.length - 1)) * 3600;
  return toInt(value) / 1000;
}

function has(args: string[], value: string): bool {
  for (let i = 0; i < args.length; i++) if (args[i] == value) return true;
  return false;
}

function toInt(value: string): i32 {
  let out = 0;
  for (let i = 0; i < value.length; i++) {
    const c = value.charCodeAt(i);
    if (c < 48 || c > 57) break;
    out = out * 10 + (c - 48);
  }
  return out;
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
