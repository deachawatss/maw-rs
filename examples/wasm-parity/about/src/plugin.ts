import { Host } from "@extism/as-pdk";
import { capture, fsList, fsRead, listSessions } from "../../../../packages/wasm-sdk/assembly";

export function myAbort(message: string | null, fileName: string | null, lineNumber: u32, columnNumber: u32): void {}

class WindowRow {
  index: string;
  name: string;

  constructor(index: string, name: string) {
    this.index = index;
    this.name = name;
  }
}

class WorktreeRow {
  name: string;
  path: string;

  constructor(name: string, path: string) {
    this.name = name;
    this.path = path;
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
  if (args.length == 0 || args[0] == "") return finish(false, null, "usage: maw about <oracle>");
  const oracle = args[0];
  const name = oracle.toLowerCase();

  const repo = readContent("/data/oracles/" + name + ".json");
  const repoPath = jsonStringField(repo, "repoPath");
  const sessionResponse = listSessions("{}");
  const session = detectSession(sessionResponse, name);
  const fleet = findFleet(name);

  if (repoPath == "" && session == "" && fleet.file == "") {
    return finish(false, null, "no oracle named '" + oracle + "' \u2014 try: maw oracle ls");
  }

  let out = "\n  \u001b[36mOracle \u2014 " + oracle + "\u001b[0m\n\n";
  out += "  Repo:      " + (repoPath == "" ? "(not found)" : repoPath) + "\n";

  const windows = session == "" ? new Array<WindowRow>() : sessionWindows(sessionResponse, session);
  if (session != "") {
    out += "  Session:   " + session + " (" + windows.length.toString() + " windows)\n";
    for (let i = 0; i < windows.length; i++) {
      const window = windows[i];
      const content = captureContent(session + ":" + window.index);
      const status = content.trim() != "" ? "\u001b[32m\u25cf\u001b[0m" : "\u001b[33m\u25cf\u001b[0m";
      out += "    " + status + " " + window.name + "\n";
    }
  } else {
    out += "  Session:   (none)\n";
  }

  const worktrees = loadWorktrees(name);
  out += "  Worktrees: " + worktrees.length.toString() + "\n";
  for (let i = 0; i < worktrees.length; i++) {
    out += "    " + worktrees[i].name + " \u2192 " + worktrees[i].path + "\n";
  }

  if (fleet.file != "") {
    out += "  Fleet:     " + fleet.file + " (" + fleet.windowCount.toString() + " registered, " + windows.length.toString() + " running)\n";
    const unregistered = unregisteredWindows(windows, fleet.windowNames);
    if (unregistered.length > 0) {
      out += "  \u001b[33m\u26a0\u001b[0m  " + unregistered.length.toString() + " window(s) not in fleet config \u2014 won't survive reboot\n";
      for (let i = 0; i < unregistered.length; i++) {
        out += "    \u001b[33m\u2192\u001b[0m " + unregistered[i] + "\n";
      }
      out += "\n  \u001b[90mFix: add to fleet/" + fleet.file + "\u001b[0m\n";
      out += "  \u001b[90m  maw fleet init          # regenerate all configs\u001b[0m\n";
      out += "  \u001b[90m  maw fleet validate      # check for problems\u001b[0m\n";
    }
  } else {
    out += "  Fleet:     (no config)\n";
  }

  return finish(true, out, null);
}

class FleetMatch {
  file: string;
  windowCount: i32;
  windowNames: string[];

  constructor(file: string, windowCount: i32, windowNames: string[]) {
    this.file = file;
    this.windowCount = windowCount;
    this.windowNames = windowNames;
  }
}

function findFleet(name: string): FleetMatch {
  const list = fsList("{\"path\":\"/config/fleet\",\"recursive\":false,\"includeDirs\":false}");
  const files = jsonArrayStrings(list, "entries");
  for (let i = 0; i < files.length; i++) {
    const body = readContent("/config/fleet/" + files[i]);
    const windows = jsonObjectsInArray(body, "windows");
    const names = new Array<string>();
    let matched = false;
    for (let j = 0; j < windows.length; j++) {
      const windowName = jsonStringField(windows[j], "name");
      names.push(windowName);
      if (windowName.toLowerCase() == name || windowName.toLowerCase() == name + "-oracle") matched = true;
    }
    if (matched) return new FleetMatch(files[i], names.length, names);
  }
  return new FleetMatch("", 0, new Array<string>());
}

function unregisteredWindows(windows: WindowRow[], registered: string[]): string[] {
  const out = new Array<string>();
  for (let i = 0; i < windows.length; i++) {
    let found = false;
    for (let j = 0; j < registered.length; j++) if (registered[j] == windows[i].name) found = true;
    if (!found) out.push(windows[i].name);
  }
  return out;
}

function detectSession(json: string, name: string): string {
  const sessions = jsonObjectsInArray(json, "sessions");
  for (let i = 0; i < sessions.length; i++) {
    const sessionName = jsonStringField(sessions[i], "name");
    if (sessionName.toLowerCase() == name) return sessionName;
  }
  for (let i = 0; i < sessions.length; i++) {
    const sessionName = jsonStringField(sessions[i], "name");
    const windows = jsonObjectsInArray(sessions[i], "windows");
    for (let j = 0; j < windows.length; j++) {
      const windowName = jsonStringField(windows[j], "name").toLowerCase();
      if (windowName == name || windowName == name + "-oracle") return sessionName;
    }
  }
  return "";
}

function sessionWindows(json: string, sessionName: string): WindowRow[] {
  const sessions = jsonObjectsInArray(json, "sessions");
  for (let i = 0; i < sessions.length; i++) {
    if (jsonStringField(sessions[i], "name") != sessionName) continue;
    const windows = jsonObjectsInArray(sessions[i], "windows");
    const out = new Array<WindowRow>();
    for (let j = 0; j < windows.length; j++) {
      out.push(new WindowRow(jsonNumberTextField(windows[j], "index"), jsonStringField(windows[j], "name")));
    }
    return out;
  }
  return new Array<WindowRow>();
}

function loadWorktrees(name: string): WorktreeRow[] {
  const list = fsList("{\"path\":\"/data/about/" + name + "/worktrees\",\"recursive\":false,\"includeDirs\":false}");
  const files = jsonArrayStrings(list, "entries");
  const rows = new Array<WorktreeRow>();
  for (let i = 0; i < files.length; i++) {
    const body = readContent("/data/about/" + name + "/worktrees/" + files[i]);
    const wtName = jsonStringField(body, "name");
    const path = jsonStringField(body, "path");
    if (wtName != "" || path != "") rows.push(new WorktreeRow(wtName, path));
  }
  return rows;
}

function captureContent(target: string): string {
  const out = capture("{\"target\":\"" + target + "\",\"lines\":3}");
  return jsonStringField(out, "content");
}

function readContent(path: string): string {
  const out = fsRead("{\"path\":" + quote(path) + ",\"encoding\":\"utf8\"}");
  if (out.indexOf("\"ok\":true") < 0) return "";
  return jsonStringField(out, "content");
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

function jsonNumberTextField(json: string, key: string): string {
  const marker = "\"" + key + "\":";
  const start = json.indexOf(marker);
  if (start < 0) return "";
  let i = start + marker.length;
  while (i < json.length && isSpace(json.charCodeAt(i))) i++;
  const begin = i;
  while (i < json.length) {
    const c = json.charCodeAt(i);
    if (c < 48 || c > 57) break;
    i++;
  }
  return json.slice(begin, i);
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

function isSpace(c: i32): bool {
  return c == 32 || c == 9 || c == 10 || c == 13;
}

function hex4(code: i32): string {
  const digits = "0123456789abcdef";
  return digits.charAt((code >> 12) & 15) + digits.charAt((code >> 8) & 15) + digits.charAt((code >> 4) & 15) + digits.charAt(code & 15);
}
