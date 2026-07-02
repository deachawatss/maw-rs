function parseFlags(args: string[]): Map<string, string> {
  const flags = new Map<string, string>();
  for (const arg of args) {
    if (!arg.startsWith("--")) continue;
    const eq = arg.indexOf("=");
    if (eq >= 0) flags.set(arg.slice(2, eq), arg.slice(eq + 1));
    else flags.set(arg.slice(2), "true");
  }
  return flags;
}

function sparkline(values: number[]): string {
  if (values.length === 0) return "";
  const ticks = "12345678";
  const min = Math.min(...values);
  const max = Math.max(...values);
  const span = max - min;
  return values.map((value) => ticks[span === 0 ? 0 : Math.trunc(((value - min) * 7) / span)]).join("");
}

function validateNickname(name: string): boolean {
  return /^[A-Za-z0-9_-]{1,32}$/.test(name);
}

function buildCommand(command: string, args: string[] = []): string {
  return [command, ...args].join(" ");
}

export default async function handle(ctx: { source: string; args: string[]; writer?: (...args: unknown[]) => void }) {
  const flags = parseFlags(["--loud", "--name=nova"]);
  const valid = validateNickname("nova-codex-1") ? "true" : "false";
  return {
    ok: true,
    output: `input=${JSON.stringify({ args: ctx.args, source: ctx.source })} name=${flags.get("name")} loud=${flags.get("loud")} valid=${valid} spark=${sparkline([1, 3, 2, 5])} cmd=${buildCommand("maw", ["ping"])} stat={"ok":true,"value":{"exists":true,"kind":"file","bytes":7}}`,
  };
}
