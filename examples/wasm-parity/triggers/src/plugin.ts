import { Host } from "@extism/as-pdk";

export function myAbort(message: string | null, fileName: string | null, lineNumber: u32, columnNumber: u32): void {}

export function handle(): i32 {
  const output = "\x1b[90mNo triggers configured. Add a 'triggers' array to maw.config.json.\x1b[0m\n" +
    "\n" +
    "\x1b[90mExample:\x1b[0m\n" +
    "  \"triggers\": [\n" +
    "    { \"on\": \"issue-close\", \"repo\": \"Soul-Brews-Studio/maw-js\", \"action\": \"maw hey pulse-oracle 'issue closed'\" },\n" +
    "    { \"on\": \"pr-merge\", \"repo\": \"Soul-Brews-Studio/maw-js\", \"action\": \"maw done neo-mawjs\" },\n" +
    "    { \"on\": \"agent-idle\", \"timeout\": 30, \"action\": \"maw sleep {agent}\" }\n" +
    "  ]";
  Host.outputString("{\"ok\":true,\"output\":" + quote(output) + "}");
  return 0;
}

function quote(value: string): string {
  let out = "\"";
  for (let i = 0; i < value.length; i++) {
    const ch = value.charAt(i);
    if (ch == "\\") out += "\\\\";
    else if (ch == "\"") out += "\\\"";
    else if (ch == "\n") out += "\\n";
    else if (ch == "\r") out += "\\r";
    else if (ch == "\t") out += "\\t";
    else if (ch.charCodeAt(0) < 32) out += "\\u" + hex4(ch.charCodeAt(0));
    else out += ch;
  }
  return out + "\"";
}

function hex4(code: i32): string { const digits = "0123456789abcdef"; return digits.charAt((code >> 12) & 15) + digits.charAt((code >> 8) & 15) + digits.charAt((code >> 4) & 15) + digits.charAt(code & 15); }
