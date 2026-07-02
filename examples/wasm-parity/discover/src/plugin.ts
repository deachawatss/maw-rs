import { Host } from "@extism/as-pdk";
import { fsList, fsRead, listSessions, loadConfig } from "../../../../packages/wasm-sdk/assembly";

export function myAbort(message: string | null, fileName: string | null, lineNumber: u32, columnNumber: u32): void {}

const USAGE = "usage: maw discover [--peers config|scout|both] [--json] [--tree] [--awake]";

class ParsedString { value: string; next: i32; constructor(value: string, next: i32) { this.value = value; this.next = next; } }
class ParsedValue { value: string; next: i32; constructor(value: string, next: i32) { this.value = value; this.next = next; } }

class Peer {
  source: string;
  name: string;
  node: string;
  oracle: string;
  url: string;
  awake: bool;
  liveTargets: string[];
  liveSessions: string[];

  constructor(source: string, name: string, node: string, oracle: string, url: string) {
    this.source = source;
    this.name = name;
    this.node = node;
    this.oracle = oracle;
    this.url = url;
    this.awake = false;
    this.liveTargets = new Array<string>();
    this.liveSessions = new Array<string>();
  }
}

class FleetRecord {
  file: string;
  slot: i32;
  groupName: string;
  session: string;
  name: string;
  repo: string;
  node: string;
  endpoint: string;
  peerMatched: bool;

  constructor(file: string, slot: i32, groupName: string, session: string, name: string, repo: string, node: string, endpoint: string, peerMatched: bool) {
    this.file = file;
    this.slot = slot;
    this.groupName = groupName;
    this.session = session;
    this.name = name;
    this.repo = repo;
    this.node = node;
    this.endpoint = endpoint;
    this.peerMatched = peerMatched;
  }
}

class LivePane {
  id: string;
  target: string;
  session: string;
  window: string;
  pane: string;
  matches: string[];

  constructor(id: string, target: string, session: string, window: string, pane: string, matches: string[]) {
    this.id = id;
    this.target = target;
    this.session = session;
    this.window = window;
    this.pane = pane;
    this.matches = matches;
  }
}

class LiveWindow {
  name: string;
  panes: LivePane[];
  constructor(name: string, panes: LivePane[]) { this.name = name; this.panes = panes; }
}

class LiveSession {
  name: string;
  windows: LiveWindow[];
  constructor(name: string, windows: LiveWindow[]) { this.name = name; this.windows = windows; }
}

export function handle(): i32 {
  const args = extractArgs(Host.inputString());
  const rawMode = readOption(args, "--peers", "config");
  if (rawMode != "config" && rawMode != "scout" && rawMode != "both") return finish(false, USAGE, "invalid_peer_source");
  if (rawMode != "config") return finish(false, USAGE, "discover wasm parity fixture supports --peers config only");

  const json = has(args, "--json");
  const tree = has(args, "--tree");
  const awake = has(args, "--awake");

  if (awake && !tree && !json) {
    const liveOnly = loadLiveState(new Array<Peer>());
    return finish(true, formatTmuxLiveState(liveOnly), null);
  }

  const configResponse = loadConfig("{}");
  if (configResponse.indexOf("\"ok\":true") < 0) return finish(false, null, hostError(configResponse, "maw.config.get failed"));
  const config = readJsonValueAtKey(configResponse, "config");
  const peers = configuredPeerTargets(config);
  const fleet = loadFleetConfigState(config, peers);
  const includeLive = json || tree || awake;
  const live = includeLive ? loadLiveState(peers) : new Array<LivePane>();
  markPeersLive(peers, live);
  const visiblePeers = awake && !tree ? awakePeers(peers) : peers;

  if (json) return finish(true, renderJson(rawMode, awake, tree, visiblePeers, fleet, live), null);
  if (tree) return finish(true, renderTree(visiblePeers, fleet, live), null);
  return finish(true, renderTable(peers, fleet), null);
}

function configuredPeerTargets(config: string): Peer[] {
  const peers = parseNamedPeers(config);
  const legacy = jsonArrayStrings(config, "peers");
  for (let i = 0; i < legacy.length; i++) {
    let duplicate = false;
    for (let j = 0; j < peers.length; j++) if (peers[j].url == legacy[i]) duplicate = true;
    if (!duplicate) peers.push(new Peer("config", "", "", "", legacy[i]));
  }
  return peers;
}

function parseNamedPeers(json: string): Peer[] {
  const out = new Array<Peer>();
  const objects = jsonObjectsInArray(json, "namedPeers");
  for (let i = 0; i < objects.length; i++) {
    out.push(new Peer("config", jsonStringField(objects[i], "name"), "", "", jsonStringField(objects[i], "url")));
  }
  return out;
}

function loadFleetConfigState(config: string, peers: Peer[]): FleetRecord[] {
  const list = fsList("{\"path\":\"/config/fleet\",\"recursive\":false,\"includeDirs\":false}");
  const files = jsonArrayStrings(list, "entries");
  const records = new Array<FleetRecord>();
  const seen = new Array<string>();
  for (let i = 0; i < files.length; i++) {
    const file = files[i];
    const body = readContent("/config/fleet/" + file);
    const sessionName = jsonStringField(body, "name");
    const windows = jsonObjectsInArray(body, "windows");
    for (let j = 0; j < windows.length; j++) {
      const windowName = jsonStringField(windows[j], "name");
      if (windowName == "") continue;
      const repo = jsonStringField(windows[j], "repo");
      const node = fleetWindowNode(config, windowName);
      const endpoint = endpointForNode(node, peers);
      const key = node + "\u0000" + windowName + "\u0000" + repo;
      if (contains(seen, key)) continue;
      seen.push(key);
      records.push(new FleetRecord(file, fileSlot(file), fileGroup(file), sessionName, windowName, repo, node, endpoint, endpoint != ""));
    }
  }
  return records;
}

function fleetWindowNode(config: string, name: string): string {
  const agents = readJsonValueAtKey(config, "agents");
  const node = jsonStringField(agents, name);
  if (node != "") return node;
  const local = jsonStringField(config, "node");
  return local == "" ? "local" : local;
}

function endpointForNode(node: string, peers: Peer[]): string {
  for (let i = 0; i < peers.length; i++) {
    if (peers[i].name == node || peers[i].node == node || peers[i].oracle == node) return peers[i].url;
  }
  return "";
}

function loadLiveState(peers: Peer[]): LivePane[] {
  const response = listSessions("{}");
  const sessions = jsonObjectsInArray(response, "sessions");
  const out = new Array<LivePane>();
  for (let i = 0; i < sessions.length; i++) {
    const session = jsonStringField(sessions[i], "name");
    const windows = jsonObjectsInArray(sessions[i], "windows");
    for (let j = 0; j < windows.length; j++) {
      const window = jsonStringField(windows[j], "name");
      const index = jsonNumberTextField(windows[j], "index");
      const pane = "0";
      const target = session + ":" + window + "." + pane;
      out.push(new LivePane(target, target, session, window, pane, liveMatches(session, window, peers)));
    }
  }
  return out;
}

function liveMatches(session: string, window: string, peers: Peer[]): string[] {
  const out = new Array<string>();
  for (let i = 0; i < peers.length; i++) {
    if (peerMatchesSignal(peers[i], session) || peerMatchesSignal(peers[i], window)) out.push(peerLabel(peers[i]));
  }
  return out;
}

function peerMatchesSignal(peer: Peer, signal: string): bool {
  const aliases = normalizedAliases(signal);
  for (let i = 0; i < aliases.length; i++) {
    if (matchesAlias(peer.name, aliases[i]) || matchesAlias(peer.node, aliases[i]) || matchesAlias(peer.oracle, aliases[i])) return true;
  }
  return false;
}

function matchesAlias(value: string, alias: string): bool {
  if (value == "") return false;
  const aliases = normalizedAliases(value);
  for (let i = 0; i < aliases.length; i++) if (aliases[i] == alias) return true;
  return false;
}

function normalizedAliases(value: string): string[] {
  const base = normalizeSignal(value);
  const out = new Array<string>();
  if (base == "") return out;
  pushUnique(out, base);
  const noPrefix = stripNumericPrefix(base);
  pushUnique(out, noPrefix);
  pushUnique(out, stripOracleSuffix(base));
  pushUnique(out, stripOracleSuffix(noPrefix));
  return out;
}

function normalizeSignal(value: string): string { return value.trim().toLowerCase(); }
function stripNumericPrefix(value: string): string {
  let i = 0;
  while (i < value.length) {
    const c = value.charCodeAt(i);
    if (c < 48 || c > 57) break;
    i++;
  }
  return i > 0 && i < value.length && value.charAt(i) == "-" ? value.slice(i + 1) : value;
}
function stripOracleSuffix(value: string): string { return value.endsWith("-oracle") ? value.slice(0, value.length - 7) : value; }

function markPeersLive(peers: Peer[], live: LivePane[]): void {
  for (let i = 0; i < peers.length; i++) {
    const peer = peers[i];
    for (let j = 0; j < live.length; j++) {
      if (contains(live[j].matches, peerLabel(peer))) {
        peer.awake = true;
        pushUnique(peer.liveTargets, live[j].target);
        pushUnique(peer.liveSessions, live[j].session);
      }
    }
  }
}

function awakePeers(peers: Peer[]): Peer[] {
  const out = new Array<Peer>();
  for (let i = 0; i < peers.length; i++) if (peers[i].awake) out.push(peers[i]);
  return out;
}

function renderTable(peers: Peer[], fleet: FleetRecord[]): string {
  const chunks = new Array<string>();
  chunks.push(formatPeerSources(peers));
  if (fleet.length > 0) chunks.push("fleet config\n" + renderFleetConfig(fleet));
  return chunks.join("\n\n");
}

function formatPeerSources(peers: Peer[]): string {
  if (peers.length == 0) return "no peers discovered or configured";
  const header = ["source", "name", "node", "oracle", "url"];
  const rows = new Array<string[]>();
  for (let i = 0; i < peers.length; i++) rows.push([peers[i].source, orDash(peers[i].name), orDash(peers[i].node), orDash(peers[i].oracle), peers[i].url]);
  return table(header, rows);
}

function renderFleetConfig(fleet: FleetRecord[]): string {
  if (fleet.length == 0) return "no configured fleet workspaces";
  const header = ["node", "name", "session", "endpoint", "repo"];
  const rows = new Array<string[]>();
  for (let i = 0; i < fleet.length; i++) rows.push([fleet[i].node, fleet[i].name, fleet[i].session, fleet[i].endpoint == "" ? "offline" : fleet[i].endpoint, orDash(fleet[i].repo)]);
  return table(header, rows);
}

function formatTmuxLiveState(live: LivePane[]): string {
  if (live.length == 0) return "no live tmux sessions/windows found";
  const header = ["source", "session", "window", "pane", "command", "cwd", "matches"];
  const rows = new Array<string[]>();
  for (let i = 0; i < live.length; i++) rows.push(["tmux", live[i].session, live[i].window, live[i].pane, "-", "-", live[i].matches.length > 0 ? live[i].matches.join(",") : "-"]);
  return table(header, rows);
}

function renderTree(peers: Peer[], fleet: FleetRecord[], live: LivePane[]): string {
  let out = "discover";
  out += "\n  tmux (" + live.length.toString() + " live pane" + (live.length == 1 ? "" : "s") + ")";
  const sessions = summarizeLiveSessions(live);
  for (let i = 0; i < sessions.length; i++) {
    out += "\n    " + sessions[i].name;
    for (let j = 0; j < sessions[i].windows.length; j++) {
      const window = sessions[i].windows[j];
      out += "\n      " + window.name;
      for (let k = 0; k < window.panes.length; k++) {
        const pane = window.panes[k];
        out += "\n        " + pane.pane + (pane.matches.length > 0 ? " matches=" + pane.matches.join(",") : "");
      }
    }
  }
  out += "\n  federation peers (" + peers.length.toString() + ")";
  for (let i = 0; i < peers.length; i++) out += "\n    config " + peerLabel(peers[i]) + " -> " + peers[i].url;
  out += "\n  fleet config (" + fleet.length.toString() + " configured)";
  for (let i = 0; i < fleet.length; i++) {
    const endpoint = fleet[i].endpoint != "" ? " endpoint=" + fleet[i].endpoint : " offline";
    const repo = fleet[i].repo != "" ? " repo=" + fleet[i].repo : "";
    out += "\n    " + fleet[i].node + "/" + fleet[i].name + " " + fleet[i].session + endpoint + repo;
  }
  out += "\n  registered oracles (0)\n  plugins (0 registered)\n  ghq (0 repos)";
  return out;
}

function renderJson(mode: string, awake: bool, tree: bool, peers: Peer[], fleet: FleetRecord[], live: LivePane[]): string {
  const liveSessions = summarizeLiveSessions(live);
  const total = tree ? peers.length + live.length + fleet.length : peers.length;
  let out = "{\n";
  out += "  \"ok\": true,\n";
  out += "  \"mode\": " + quote(mode) + ",\n";
  out += "  \"total\": " + total.toString() + ",\n";
  out += "  \"awake\": " + boolJson(awake) + ",\n";
  out += "  \"awakeOnly\": " + boolJson(awake) + ",\n";
  out += "  \"peers\": " + peersJson(peers, 2) + ",\n";
  out += "  \"fleet\": {\n    \"source\": \"fleet-config\",\n    \"total\": " + fleet.length.toString() + ",\n    \"records\": " + fleetJson(fleet, 2) + "\n  },\n";
  out += "  \"oracles\": {\n    \"source\": \"oracle-manifest\",\n    \"total\": 0,\n    \"records\": []\n  },\n";
  out += "  \"plugins\": {\n    \"source\": \"plugin-registry\",\n    \"total\": 0,\n    \"records\": []\n  },\n";
  out += "  \"ghq\": {\n    \"source\": \"ghq\",\n    \"total\": 0,\n    \"repos\": []\n  },\n";
  out += "  \"liveTotal\": " + live.length.toString() + ",\n";
  out += "  \"live\": {\n    \"source\": \"tmux\",\n    \"total\": " + live.length.toString() + ",\n    \"panes\": " + livePanesJson(live, 2) + ",\n    \"sessions\": " + liveSessionsJson(liveSessions, 2) + "\n  }";
  if (tree) {
    out += ",\n  \"tree\": {\n";
    out += "    \"live\": " + liveSessionsJson(liveSessions, 2) + ",\n";
    out += "    \"peers\": " + peersJson(peers, 2) + ",\n";
    out += "    \"fleet\": " + fleetJson(fleet, 2) + ",\n";
    out += "    \"oracles\": [],\n    \"plugins\": [],\n    \"ghq\": []\n  }";
  }
  out += ",\n  \"warnings\": []\n}";
  return out;
}

function peersJson(peers: Peer[], level: i32): string {
  if (peers.length == 0) return "[]";
  const pad = indent(level);
  let out = "[";
  for (let i = 0; i < peers.length; i++) {
    const p = peers[i];
    out += "\n" + pad + "{\n";
    out += pad + "  \"source\": " + quote(p.source) + ",\n";
    if (p.name != "") out += pad + "  \"name\": " + quote(p.name) + ",\n";
    if (p.node != "") out += pad + "  \"node\": " + quote(p.node) + ",\n";
    if (p.oracle != "") out += pad + "  \"oracle\": " + quote(p.oracle) + ",\n";
    out += pad + "  \"url\": " + quote(p.url) + ",\n";
    out += pad + "  \"awake\": " + boolJson(p.awake) + ",\n";
    out += pad + "  \"liveTargets\": " + stringArrayJson(p.liveTargets) + ",\n";
    out += pad + "  \"liveSessions\": " + stringArrayJson(p.liveSessions) + "\n";
    out += pad + "}" + (i + 1 == peers.length ? "" : ",");
  }
  out += "\n" + indent(level - 1) + "]";
  return out;
}

function fleetJson(fleet: FleetRecord[], level: i32): string {
  if (fleet.length == 0) return "[]";
  const pad = indent(level);
  let out = "[";
  for (let i = 0; i < fleet.length; i++) {
    const f = fleet[i];
    out += "\n" + pad + "{\n";
    out += pad + "  \"source\": \"fleet-config\",\n";
    out += pad + "  \"type\": \"workspace\",\n";
    out += pad + "  \"file\": " + quote(f.file) + ",\n";
    out += pad + "  \"slot\": " + f.slot.toString() + ",\n";
    out += pad + "  \"groupName\": " + quote(f.groupName) + ",\n";
    out += pad + "  \"session\": " + quote(f.session) + ",\n";
    out += pad + "  \"name\": " + quote(f.name) + ",\n";
    if (f.repo != "") out += pad + "  \"repo\": " + quote(f.repo) + ",\n";
    out += pad + "  \"node\": " + quote(f.node) + ",\n";
    if (f.endpoint != "") out += pad + "  \"endpoint\": " + quote(f.endpoint) + ",\n";
    out += pad + "  \"peerMatched\": " + boolJson(f.peerMatched) + "\n";
    out += pad + "}" + (i + 1 == fleet.length ? "" : ",");
  }
  out += "\n" + indent(level - 1) + "]";
  return out;
}

function livePanesJson(live: LivePane[], level: i32): string {
  if (live.length == 0) return "[]";
  const pad = indent(level);
  let out = "[";
  for (let i = 0; i < live.length; i++) {
    const p = live[i];
    out += "\n" + pad + "{\n";
    out += pad + "  \"source\": \"tmux\",\n";
    out += pad + "  \"id\": " + quote(p.id) + ",\n";
    out += pad + "  \"target\": " + quote(p.target) + ",\n";
    out += pad + "  \"session\": " + quote(p.session) + ",\n";
    out += pad + "  \"window\": " + quote(p.window) + ",\n";
    out += pad + "  \"pane\": " + quote(p.pane) + ",\n";
    out += pad + "  \"awake\": true,\n";
    out += pad + "  \"matches\": " + stringArrayJson(p.matches) + "\n";
    out += pad + "}" + (i + 1 == live.length ? "" : ",");
  }
  out += "\n" + indent(level - 1) + "]";
  return out;
}

function liveSessionsJson(sessions: LiveSession[], level: i32): string {
  if (sessions.length == 0) return "[]";
  const pad = indent(level);
  let out = "[";
  for (let i = 0; i < sessions.length; i++) {
    const s = sessions[i];
    const paneCount = sessionPaneCount(s);
    out += "\n" + pad + "{\n";
    out += pad + "  \"source\": \"tmux\",\n";
    out += pad + "  \"name\": " + quote(s.name) + ",\n";
    out += pad + "  \"awake\": true,\n";
    out += pad + "  \"paneCount\": " + paneCount.toString() + ",\n";
    out += pad + "  \"windows\": " + liveWindowsJson(s.windows, level + 1) + "\n";
    out += pad + "}" + (i + 1 == sessions.length ? "" : ",");
  }
  out += "\n" + indent(level - 1) + "]";
  return out;
}

function liveWindowsJson(windows: LiveWindow[], level: i32): string {
  if (windows.length == 0) return "[]";
  const pad = indent(level);
  let out = "[";
  for (let i = 0; i < windows.length; i++) {
    out += "\n" + pad + "{\n";
    out += pad + "  \"name\": " + quote(windows[i].name) + ",\n";
    out += pad + "  \"paneCount\": " + windows[i].panes.length.toString() + ",\n";
    out += pad + "  \"panes\": " + livePanesJson(windows[i].panes, level + 1) + "\n";
    out += pad + "}" + (i + 1 == windows.length ? "" : ",");
  }
  out += "\n" + indent(level - 1) + "]";
  return out;
}

function summarizeLiveSessions(live: LivePane[]): LiveSession[] {
  const sessions = new Array<LiveSession>();
  for (let i = 0; i < live.length; i++) {
    let sessionIndex = -1;
    for (let s = 0; s < sessions.length; s++) if (sessions[s].name == live[i].session) sessionIndex = s;
    if (sessionIndex < 0) {
      sessions.push(new LiveSession(live[i].session, new Array<LiveWindow>()));
      sessionIndex = sessions.length - 1;
    }
    const windows = sessions[sessionIndex].windows;
    let windowIndex = -1;
    for (let w = 0; w < windows.length; w++) if (windows[w].name == live[i].window) windowIndex = w;
    if (windowIndex < 0) {
      windows.push(new LiveWindow(live[i].window, new Array<LivePane>()));
      windowIndex = windows.length - 1;
    }
    windows[windowIndex].panes.push(live[i]);
  }
  return sessions;
}

function sessionPaneCount(session: LiveSession): i32 {
  let total = 0;
  for (let i = 0; i < session.windows.length; i++) total += session.windows[i].panes.length;
  return total;
}

function table(header: string[], rows: string[][]): string {
  const widths = new Array<i32>();
  for (let i = 0; i < header.length; i++) {
    let width = header[i].length;
    for (let j = 0; j < rows.length; j++) if (rows[j][i].length > width) width = rows[j][i].length;
    widths.push(width);
  }
  const lines = new Array<string>();
  lines.push(formatColumns(header, widths));
  const div = new Array<string>();
  for (let i = 0; i < widths.length; i++) div.push(repeat("-", widths[i]));
  lines.push(formatColumns(div, widths));
  for (let i = 0; i < rows.length; i++) lines.push(formatColumns(rows[i], widths));
  return lines.join("\n");
}

function formatColumns(cols: string[], widths: i32[]): string {
  const out = new Array<string>();
  for (let i = 0; i < cols.length; i++) out.push(padRight(cols[i], widths[i]));
  return out.join("  ");
}

function readContent(path: string): string {
  const out = fsRead("{\"path\":" + quote(path) + ",\"encoding\":\"utf8\"}");
  if (out.indexOf("\"ok\":true") < 0) return "";
  return jsonStringField(out, "content");
}

function fileSlot(file: string): i32 {
  let out = 0;
  for (let i = 0; i < file.length; i++) {
    const c = file.charCodeAt(i);
    if (c < 48 || c > 57) break;
    out = out * 10 + (c - 48);
  }
  return out;
}

function fileGroup(file: string): string {
  let start = 0;
  while (start < file.length) {
    const c = file.charCodeAt(start);
    if (c < 48 || c > 57) break;
    start++;
  }
  if (start < file.length && file.charAt(start) == "-") start++;
  let end = file.length;
  if (file.endsWith(".json")) end -= 5;
  return file.slice(start, end);
}

function readOption(args: string[], name: string, fallback: string): string {
  const prefix = name + "=";
  for (let i = 0; i < args.length; i++) {
    if (args[i].startsWith(prefix)) return args[i].slice(prefix.length);
    if (args[i] == name && i + 1 < args.length && !args[i + 1].startsWith("--")) return args[i + 1];
  }
  return fallback;
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
  const i = json.indexOf(marker);
  if (i < 0) return out;
  let j = i + marker.length;
  while (j < json.length && json.charAt(j) != "[") j++;
  if (j >= json.length) return out;
  j++;
  while (j < json.length && json.charAt(j) != "]") {
    if (json.charAt(j) == "\"") {
      const parsed = readJsonString(json, j);
      out.push(parsed.value);
      j = parsed.next;
    } else {
      j++;
    }
  }
  return out;
}

function jsonStringField(json: string, key: string): string {
  const marker = "\"" + key + "\":";
  const i = json.indexOf(marker);
  if (i < 0) return "";
  let j = i + marker.length;
  while (j < json.length && json.charAt(j) != "\"") j++;
  if (j >= json.length) return "";
  return readJsonString(json, j).value;
}

function jsonNumberTextField(json: string, key: string): string {
  const marker = "\"" + key + "\":";
  const i = json.indexOf(marker);
  if (i < 0) return "0";
  let j = i + marker.length;
  while (j < json.length && isSpace(json.charCodeAt(j))) j++;
  const start = j;
  while (j < json.length) {
    const c = json.charCodeAt(j);
    if (c < 48 || c > 57) break;
    j++;
  }
  return j == start ? "0" : json.slice(start, j);
}

function readJsonValueAtKey(json: string, key: string): string {
  const marker = "\"" + key + "\":";
  const i = json.indexOf(marker);
  if (i < 0) return "{}";
  let j = i + marker.length;
  while (j < json.length && isSpace(json.charCodeAt(j))) j++;
  return readJsonValue(json, j).value;
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

function hostError(response: string, fallback: string): string { const error = jsonStringField(response, "error"); return error == "" ? fallback : error; }
function has(args: string[], value: string): bool { for (let i = 0; i < args.length; i++) if (args[i] == value) return true; return false; }
function contains(values: string[], value: string): bool { for (let i = 0; i < values.length; i++) if (values[i] == value) return true; return false; }
function pushUnique(values: string[], value: string): void { if (value != "" && !contains(values, value)) values.push(value); }
function peerLabel(peer: Peer): string { return peer.name != "" ? peer.name : peer.node != "" ? peer.node : peer.oracle != "" ? peer.oracle : "-"; }
function orDash(value: string): string { return value == "" ? "-" : value; }
function boolJson(value: bool): string { return value ? "true" : "false"; }
function isSpace(c: i32): bool { return c == 32 || c == 9 || c == 10 || c == 13; }
function repeat(value: string, count: i32): string { let out = ""; for (let i = 0; i < count; i++) out += value; return out; }
function padRight(value: string, width: i32): string { return value + repeat(" ", width - value.length); }
function indent(level: i32): string { return repeat("  ", level); }
function stringArrayJson(values: string[]): string { if (values.length == 0) return "[]"; let out = "["; for (let i = 0; i < values.length; i++) out += (i == 0 ? "" : ", ") + quote(values[i]); return out + "]"; }

function finish(ok: bool, output: string | null, error: string | null): i32 { Host.outputString(resultJson(ok, output, error)); return 0; }
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
