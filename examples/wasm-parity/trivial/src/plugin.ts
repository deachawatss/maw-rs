import { Host } from "@extism/as-pdk";
import { parseFlags, sparkline, validateNickname, buildCommand, fsStat } from "../../../../packages/wasm-sdk/assembly";

export function myAbort(message: string | null, fileName: string | null, lineNumber: u32, columnNumber: u32): void {}

export function handle(): i32 {
  const input = Host.inputString();
  const flags = parseFlags(["--loud", "--name=nova"]);
  const valid = validateNickname("nova-codex-1") ? "true" : "false";
  const stat = fsStat("{\"path\":\"/fixture.txt\"}");
  const output = "input=" + input + " name=" + flags.get("name") + " loud=" + flags.get("loud") + " valid=" + valid + " spark=" + sparkline([1, 3, 2, 5]) + " cmd=" + buildCommand("maw", ["ping"]) + " stat=" + stat;
  Host.outputString("{\"ok\":true,\"output\":" + quote(output) + "}");
  return 0;
}

function quote(value: string): string {
  let out = "\"";
  for (let i = 0; i < value.length; i++) {
    const ch = value.charAt(i);
    if (ch == "\\") out += "\\\\";
    else if (ch == "\"") out += "\\\"";
    else out += ch;
  }
  return out + "\"";
}
