import { Host } from "@extism/as-pdk";
import { fsRead, fsWrite } from "../../../../packages/wasm-sdk/assembly";

export function myAbort(message: string | null, fileName: string | null, lineNumber: u32, columnNumber: u32): void {}

const CONTACTS = "/data/contacts.json";
const FIXED_UPDATED = "2026-06-24T00:00:00.000Z";

class ContactRow {
  name: string;
  maw: string;
  thread: string;
  inbox: string;
  repo: string;
  notes: string;
  retired: bool;

  constructor(name: string, maw: string, thread: string, inbox: string, repo: string, notes: string, retired: bool) {
    this.name = name;
    this.maw = maw;
    this.thread = thread;
    this.inbox = inbox;
    this.repo = repo;
    this.notes = notes;
    this.retired = retired;
  }
}

export function handle(): i32 {
  const args = extractArgs(Host.inputString());
  const sub = args.length > 0 ? args[0].toLowerCase() : "";

  if (sub == "add" && args.length > 1) return addContact(args[1], args.slice(2));
  if (sub == "rm" || sub == "remove") {
    if (args.length < 2 || args[1] == "") return finish(false, null, "name required");
    return retireContact(args[1]);
  }
  return finish(true, formatContacts(loadContacts()), null);
}

function addContact(name: string, args: string[]): i32 {
  const rows = loadContacts();
  let found = false;
  for (let i = 0; i < rows.length; i++) {
    if (rows[i].name == name) {
      applyFlags(rows[i], args);
      rows[i].retired = false;
      found = true;
    }
  }
  if (!found) {
    const row = new ContactRow(name, "", "", "", "", "", false);
    applyFlags(row, args);
    rows.push(row);
  }
  saveContacts(rows);
  return finish(true, "\u001b[32m\u2713\u001b[0m contact \u001b[33m" + name + "\u001b[0m saved", null);
}

function retireContact(name: string): i32 {
  const rows = loadContacts();
  for (let i = 0; i < rows.length; i++) {
    if (rows[i].name == name) {
      rows[i].retired = true;
      saveContacts(rows);
      return finish(true, "\u001b[32m\u2713\u001b[0m contact \u001b[33m" + name + "\u001b[0m retired", null);
    }
  }
  return finish(true, "\u001b[31merror\u001b[0m: contact '" + name + "' not found", null);
}

function applyFlags(row: ContactRow, args: string[]): void {
  const maw = flagValue(args, "--maw");
  const thread = flagValue(args, "--thread");
  const inbox = flagValue(args, "--inbox");
  const repo = flagValue(args, "--repo");
  const notes = flagValue(args, "--notes");
  if (maw != "") row.maw = maw;
  if (thread != "") row.thread = thread;
  if (inbox != "") row.inbox = inbox;
  if (repo != "") row.repo = repo;
  if (notes != "") row.notes = notes;
}

function flagValue(args: string[], flag: string): string {
  const prefix = flag + "=";
  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg == flag && i + 1 < args.length) return args[i + 1];
    if (arg.startsWith(prefix)) return arg.slice(prefix.length);
  }
  return "";
}

function loadContacts(): ContactRow[] {
  const content = readContent(CONTACTS);
  if (content == "") return new Array<ContactRow>();
  return parseContacts(content);
}

function saveContacts(rows: ContactRow[]): void {
  fsWrite("{\"path\":\"" + CONTACTS + "\",\"content\":" + quote(contactsJson(rows)) + ",\"mode\":\"overwrite\",\"mkdirp\":true}");
}

function readContent(path: string): string {
  const out = fsRead("{\"path\":\"" + path + "\",\"encoding\":\"utf8\"}");
  if (out.indexOf("\"ok\":true") < 0) return "";
  return jsonStringField(out, "content");
}

function formatContacts(rows: ContactRow[]): string {
  const active = new Array<ContactRow>();
  for (let i = 0; i < rows.length; i++) if (!rows[i].retired) active.push(rows[i]);
  if (active.length == 0) return "\u001b[90mno contacts\u001b[0m";

  let out = "\n\u001b[36mCONTACTS\u001b[0m (" + active.length.toString() + "):\n\n";
  for (let i = 0; i < active.length; i++) {
    const c = active[i];
    const parts = new Array<string>();
    if (c.maw != "") parts.push("maw: \u001b[33m" + c.maw + "\u001b[0m");
    if (c.thread != "") parts.push("thread: \u001b[90m" + c.thread + "\u001b[0m");
    if (c.inbox != "") parts.push("inbox: \u001b[90m" + c.inbox + "\u001b[0m");
    if (c.repo != "") parts.push("repo: \u001b[90m" + c.repo + "\u001b[0m");
    if (c.notes != "") parts.push("\u001b[90m\"" + c.notes + "\"\u001b[0m");
    out += "  \u001b[32m" + pad(c.name, 12) + "\u001b[0m  " + parts.join("    ");
    if (i + 1 < active.length) out += "\n";
  }
  return out + "\n";
}

function parseContacts(json: string): ContactRow[] {
  const rows = new Array<ContactRow>();
  const object = jsonObjectField(json, "contacts");
  if (object == "") return rows;
  let i = 1;
  while (i < object.length - 1) {
    while (i < object.length && (isSpace(object.charCodeAt(i)) || object.charAt(i) == ",")) i++;
    if (i >= object.length || object.charAt(i) != "\"") break;
    const name = readJsonString(object, i);
    i = name.next;
    while (i < object.length && object.charAt(i) != "{") i++;
    if (i >= object.length) break;
    const end = matchingBrace(object, i);
    if (end <= i) break;
    const contact = object.slice(i, end + 1);
    rows.push(new ContactRow(
      name.value,
      jsonStringField(contact, "maw"),
      jsonStringField(contact, "thread"),
      jsonStringField(contact, "inbox"),
      jsonStringField(contact, "repo"),
      jsonStringField(contact, "notes"),
      jsonBoolField(contact, "retired"),
    ));
    i = end + 1;
  }
  return rows;
}

function contactsJson(rows: ContactRow[]): string {
  let out = "{\n  \"contacts\": {";
  if (rows.length > 0) out += "\n";
  for (let i = 0; i < rows.length; i++) {
    out += "    " + quote(rows[i].name) + ": " + contactJson(rows[i]);
    if (i + 1 < rows.length) out += ",";
    out += "\n";
  }
  out += "  },\n  \"updated\": " + quote(FIXED_UPDATED) + "\n}\n";
  return out;
}

function contactJson(row: ContactRow): string {
  const parts = new Array<string>();
  if (row.maw != "") parts.push("\"maw\": " + quote(row.maw));
  if (row.thread != "") parts.push("\"thread\": " + quote(row.thread));
  if (row.inbox != "") parts.push("\"inbox\": " + quote(row.inbox));
  if (row.repo != "") parts.push("\"repo\": " + quote(row.repo));
  if (row.notes != "") parts.push("\"notes\": " + quote(row.notes));
  if (row.retired) parts.push("\"retired\": true");
  if (parts.length == 0) return "{}";
  return "{\n      " + parts.join(",\n      ") + "\n    }";
}

function jsonObjectField(json: string, key: string): string {
  const marker = "\"" + key + "\":";
  const start = json.indexOf(marker);
  if (start < 0) return "";
  let i = start + marker.length;
  while (i < json.length && json.charAt(i) != "{") i++;
  if (i >= json.length) return "";
  const end = matchingBrace(json, i);
  return end > i ? json.slice(i, end + 1) : "";
}

function jsonStringField(json: string, key: string): string {
  const marker = "\"" + key + "\":";
  const start = json.indexOf(marker);
  if (start < 0) return "";
  let i = start + marker.length;
  while (i < json.length && isSpace(json.charCodeAt(i))) i++;
  if (i >= json.length || json.charAt(i) != "\"") return "";
  return readJsonString(json, i).value;
}

function jsonBoolField(json: string, key: string): bool {
  const marker = "\"" + key + "\":";
  const start = json.indexOf(marker);
  if (start < 0) return false;
  let i = start + marker.length;
  while (i < json.length && isSpace(json.charCodeAt(i))) i++;
  return json.slice(i, i + 4) == "true";
}

function matchingBrace(s: string, start: i32): i32 {
  let depth = 0;
  let inString = false;
  let escaped = false;
  for (let i = start; i < s.length; i++) {
    const ch = s.charAt(i);
    if (inString) {
      if (escaped) escaped = false;
      else if (ch == "\\") escaped = true;
      else if (ch == "\"") inString = false;
    } else if (ch == "\"") inString = true;
    else if (ch == "{") depth++;
    else if (ch == "}") {
      depth--;
      if (depth == 0) return i;
    }
  }
  return -1;
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

class ParsedString {
  value: string;
  next: i32;
  constructor(value: string, next: i32) {
    this.value = value;
    this.next = next;
  }
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

function pad(value: string, width: i32): string {
  let out = value;
  while (out.length < width) out += " ";
  return out;
}

function isSpace(c: i32): bool {
  return c == 32 || c == 9 || c == 10 || c == 13;
}

function hex4(code: i32): string {
  const digits = "0123456789abcdef";
  return digits.charAt((code >> 12) & 15) + digits.charAt((code >> 8) & 15) + digits.charAt((code >> 4) & 15) + digits.charAt(code & 15);
}
