#![allow(
    clippy::pedantic,
    clippy::module_name_repetitions,
    clippy::too_many_lines
)]

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    env, fs,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const DISCORD_API_BASE: &str = "https://discord.com/api/v10";
const VERSION: &str = "0.4.2";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscordOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiscordHttpResponse {
    pub status: u16,
    pub body: Value,
    pub retry_after: Option<f64>,
}

pub trait DiscordRest: Send + Sync {
    fn get_json<'a>(
        &'a self,
        path: &'a str,
        token: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<DiscordHttpResponse, String>> + Send + 'a>>;
}

#[derive(Debug, Clone)]
pub struct ReqwestDiscordRest {
    client: reqwest::Client,
    base: &'static str,
}

impl ReqwestDiscordRest {
    /// Build a rustls-only Discord REST client pinned to `discord.com/api/v10`.
    ///
    /// The base URL is not configurable by callers; all paths are appended only
    /// after rejecting absolute URLs and non-leading-slash values.
    ///
    /// # Errors
    ///
    /// Returns the reqwest builder error if the TLS/client setup fails.
    pub fn new() -> Result<Self, reqwest::Error> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            base: DISCORD_API_BASE,
        })
    }

    fn url_for(&self, path: &str) -> Result<String, String> {
        if !path.starts_with('/') || path.starts_with("//") || path.contains("://") {
            return Err("Discord REST path must be host-relative".to_owned());
        }
        Ok(format!("{}{}", self.base, path))
    }
}

impl DiscordRest for ReqwestDiscordRest {
    fn get_json<'a>(
        &'a self,
        path: &'a str,
        token: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<DiscordHttpResponse, String>> + Send + 'a>> {
        Box::pin(async move {
            let url = self.url_for(path)?;
            let res = self
                .client
                .get(url)
                .header(reqwest::header::AUTHORIZATION, format!("Bot {token}"))
                .send()
                .await
                .map_err(|_| "Discord REST request failed".to_owned())?;
            let status = res.status().as_u16();
            let retry_after = res
                .headers()
                .get("retry-after")
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.parse::<f64>().ok());
            let body = res.json::<Value>().await.unwrap_or(Value::Null);
            Ok(DiscordHttpResponse {
                status,
                body,
                retry_after,
            })
        })
    }
}

#[derive(Debug, Clone)]
pub struct DiscordEnv {
    pub home: PathBuf,
    pub ghq_root: PathBuf,
    pub hostname: String,
}

impl DiscordEnv {
    #[must_use]
    pub fn from_process() -> Self {
        let home = env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
        let ghq_root = env::var_os("GHQ_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.clone());
        let hostname = env::var("HOSTNAME")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                Command::new("hostname")
                    .output()
                    .ok()
                    .and_then(|out| String::from_utf8(out.stdout).ok())
                    .map_or_else(|| "unknown".to_owned(), |s| s.trim().to_owned())
            });
        Self {
            home,
            ghq_root,
            hostname,
        }
    }

    fn pass_dir(&self) -> PathBuf {
        self.home.join(".password-store/discord")
    }

    fn legacy_state_root(&self) -> PathBuf {
        self.home.join(".claude/channels")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenEntry {
    pub name: String,
    pub bot: String,
    pub file: PathBuf,
    pub size_bytes: u64,
    pub modified: Option<SystemTime>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct AccessFile {
    #[serde(default = "default_dm_policy")]
    dm_policy: String,
    #[serde(default)]
    allow_from: Vec<String>,
    #[serde(default)]
    groups: BTreeMap<String, AccessGroup>,
    #[serde(default)]
    pending: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct AccessGroup {
    #[serde(default = "default_true")]
    require_mention: bool,
    #[serde(default)]
    allow_from: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Guild {
    id: String,
    name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Channel {
    id: String,
    name: String,
    #[serde(rename = "type")]
    kind: u8,
    #[serde(default)]
    parent_id: Option<String>,
    #[serde(default)]
    guild_id: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_dm_policy() -> String {
    "allowlist".to_owned()
}

/// Run the native Discord REST plugin command.
///
/// # Errors
///
/// Returns an error output if the reqwest client cannot be constructed.
pub async fn run_discord_command(args: Vec<String>) -> DiscordOutput {
    let Ok(rest) = ReqwestDiscordRest::new() else {
        return DiscordOutput {
            code: 1,
            stdout: String::new(),
            stderr: "failed to initialize Discord REST client\n".to_owned(),
        };
    };
    run_discord_command_with(&args, &DiscordEnv::from_process(), &rest).await
}

pub async fn run_discord_command_with(
    args: &[String],
    env: &DiscordEnv,
    rest: &dyn DiscordRest,
) -> DiscordOutput {
    let mut logs = Vec::new();
    let ok = match args.first().map(|s| s.to_lowercase()) {
        None => {
            usage(&mut logs);
            true
        }
        Some(sub) if matches!(sub.as_str(), "help" | "-h" | "--help") => {
            usage(&mut logs);
            true
        }
        Some(sub) if matches!(sub.as_str(), "version" | "-v" | "--version") => {
            version(&mut logs);
            true
        }
        Some(sub) if sub == "tokens" => tokens(env, rest, &args[1..], &mut logs).await,
        Some(sub) if sub == "status" => status(env, rest, &args[1..], &mut logs).await,
        Some(sub) if sub == "bind" => bind(env, &args[1..], &mut logs),
        Some(sub) if sub == "access" => access(env, rest, &args[1..], &mut logs).await,
        Some(sub) if sub == "guilds" => guilds(env, rest, &args[1..], &mut logs).await,
        Some(sub) if sub == "channels" => channels(env, rest, &args[1..], &mut logs).await,
        Some(sub) if sub == "members" => members(env, rest, &args[1..], &mut logs).await,
        Some(sub) if sub == "inventory" => inventory(env, rest, &args[1..], &mut logs).await,
        Some(sub) if matches!(sub.as_str(), "pair" | "route" | "serve") => {
            logs.push(format!(
                "✗ '{sub}' not implemented yet (v0.4 ships tokens + status + bind + access)."
            ));
            logs.push("planned for v0.5 — see 'maw discord' for full subcommand list.".to_owned());
            false
        }
        Some(sub) => {
            logs.push(format!("unknown subcommand: {sub}"));
            usage(&mut logs);
            false
        }
    };

    DiscordOutput {
        code: if ok { 0 } else { 1 },
        stdout: with_final_newline(&logs.join("\n")),
        stderr: String::new(),
    }
}

fn with_final_newline(s: &str) -> String {
    if s.is_empty() {
        String::new()
    } else if s.ends_with('\n') {
        s.to_owned()
    } else {
        format!("{s}\n")
    }
}

fn usage(log: &mut Vec<String>) {
    log.extend([
        "usage: maw discord <subcommand> [args]".to_owned(),
        String::new(),
        "subcommands:".to_owned(),
        "  version                            show plugin version + subcommand status".to_owned(),
        "  tokens ls                          list all Discord bot tokens in pass (no reveal)".to_owned(),
        "  tokens check [bot]                 verify each token decrypts + Discord REST 200".to_owned(),
        "  status [bot] [--check] [--redact] [--json]".to_owned(),
        "                                     fleet inspection from this host — pass × legacy × hybrid × tmux × registry".to_owned(),
        "  bind <bot> [--apply] [--restart] [--session <name>] [--force]".to_owned(),
        "                                     end-to-end Discord-online for a bot on this host".to_owned(),
        "  access <bot> <list|show|map|add|rm|set|allow|lockdown> [...]".to_owned(),
        "                                     channel + allowlist management per bot (NEW v0.4)".to_owned(),
        String::new(),
        "subcommands (planned):".to_owned(),
        "  pair <oracle> <channel>            access.json + channel-map.json bootstrap (v0.5)".to_owned(),
        "  route <from> <to>                  channel-map.json entry (v0.5)".to_owned(),
        "  serve (hook handler)               wires after_send → Discord post (v0.5)".to_owned(),
        String::new(),
        "token strategy: HYBRID — tokens in pass (central), .discord/ config in bot repo.".to_owned(),
        "see: ψ/outbox/ideas/2026-05-17_self-contained-bot-repo-gpg-pattern.md".to_owned(),
    ]);
}

fn version(log: &mut Vec<String>) {
    log.extend([
        format!("maw discord v{VERSION}"),
        String::new(),
        "subcommand status:".to_owned(),
        "  ✓ tokens ls / check        v0.1".to_owned(),
        "  ✓ status [bot] [flags]     v0.3.1 (real online/where via bun ancestry)".to_owned(),
        "  ✓ bind <bot>               v0.3 (rewrite to use 'maw wake' pending)".to_owned(),
        "  ✓ access <bot> ...         v0.4 (list/show/map/add/rm/set/allow/lockdown)".to_owned(),
        "  ✓ guilds/channels/members/inventory <bot>  v0.4.2 (Discord-state visibility)".to_owned(),
        "  ⏸ pair <oracle> <chan>     v0.5 planned".to_owned(),
        "  ⏸ route <from> <to>        v0.5 planned".to_owned(),
        "  ⏸ serve (after_send hook)  v0.5 planned (replaces daemon — engine.serve)".to_owned(),
    ]);
}

fn list_pass_tokens(env: &DiscordEnv) -> Vec<TokenEntry> {
    let dir = env.pass_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?.strip_suffix(".gpg")?.to_owned();
            let meta = entry.metadata().ok()?;
            let bot = name.strip_suffix("-token").unwrap_or(&name).to_owned();
            Some(TokenEntry {
                name,
                bot,
                file: path,
                size_bytes: meta.len(),
                modified: meta.modified().ok(),
            })
        })
        .collect::<Vec<_>>();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn decrypt_token(name: &str) -> Option<String> {
    if let Ok(token) = env::var("DISCORD_BOT_TOKEN") {
        let trimmed = token.trim().to_owned();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }
    if rejects_option_arg(name) || name.contains('/') || name.contains("..") {
        return None;
    }
    let out = Command::new("pass")
        .args(["show", &format!("discord/{name}")])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let token = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!token.is_empty()).then_some(token)
}

fn rejects_option_arg(value: &str) -> bool {
    value == "--" || value.starts_with('-')
}

async fn ping(rest: &dyn DiscordRest, token: &str) -> (bool, u16, Option<String>) {
    match rest.get_json("/users/@me", token).await {
        Ok(res) if (200..300).contains(&res.status) => (
            true,
            res.status,
            res.body
                .get("username")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        ),
        Ok(res) => (false, res.status, None),
        Err(_) => (false, 0, None),
    }
}

async fn tokens(
    env: &DiscordEnv,
    rest: &dyn DiscordRest,
    args: &[String],
    log: &mut Vec<String>,
) -> bool {
    let action = args.first().map_or("ls", String::as_str).to_lowercase();
    match action.as_str() {
        "ls" => tokens_ls(env, log),
        "check" => tokens_check(env, rest, args.get(1).map(String::as_str), log).await,
        _ => {
            log.push(format!("unknown subcommand: tokens {action}"));
            log.push("usage: maw discord tokens <ls|check> [bot]".to_owned());
            false
        }
    }
}

fn tokens_ls(env: &DiscordEnv, log: &mut Vec<String>) -> bool {
    let tokens = list_pass_tokens(env);
    if tokens.is_empty() {
        log.push("✗ no tokens in pass (~/.password-store/discord/)".to_owned());
        log.push("hint: pass insert discord/<bot>-token".to_owned());
        return true;
    }
    log.push(format!(
        "📦 {} token(s) in pass (~/.password-store/discord/)",
        tokens.len()
    ));
    log.push(String::new());
    log.push("  name                                  size    last-modified".to_owned());
    log.push("  ──────────────────────────────────────────────────────────────".to_owned());
    for token in tokens {
        log.push(format!(
            "  {:<38}{:<7} {}",
            token.name,
            format!("{}B", token.size_bytes),
            token.modified.map_or_else(|| "—".to_owned(), ymd_utc)
        ));
    }
    log.push(String::new());
    log.push("use 'maw discord tokens check' to verify each one decrypts + Discord 200".to_owned());
    true
}

async fn tokens_check(
    env: &DiscordEnv,
    rest: &dyn DiscordRest,
    only: Option<&str>,
    log: &mut Vec<String>,
) -> bool {
    let tokens = list_pass_tokens(env);
    if tokens.is_empty() {
        log.push("✗ no tokens to check".to_owned());
        return true;
    }
    let filtered = tokens
        .into_iter()
        .filter(|t| {
            only.is_none_or(|needle| {
                t.name == needle || t.name == format!("{needle}-token") || t.bot == needle
            })
        })
        .collect::<Vec<_>>();
    if filtered.is_empty() {
        let needle = only.unwrap_or_default();
        log.push(format!(
            "✗ no token matching '{needle}' (tried '{needle}', '{needle}-token', bot=='{needle}')"
        ));
        return true;
    }
    log.push(format!("🔐 checking {} token(s)...", filtered.len()));
    log.push(String::new());
    log.push("  name                                  decrypt  discord  bot".to_owned());
    log.push("  ──────────────────────────────────────────────────────────────────".to_owned());
    let mut ok_count = 0;
    let mut fail_count = 0;
    for entry in &filtered {
        let name = format!("{:<38}", entry.name);
        let Some(token) = decrypt_token(&entry.name) else {
            log.push(format!("  {name}✗ fail   —        —"));
            fail_count += 1;
            continue;
        };
        let (ok, status, username) = ping(rest, &token).await;
        let status_text = if ok {
            format!("✓ {status}    ")
        } else if status == 0 {
            "✗ ERR   ".to_owned()
        } else {
            format!("✗ {status}   ")
        };
        log.push(format!(
            "  {name}✓ OK    {status_text} {}",
            username.unwrap_or_else(|| "—".to_owned())
        ));
        if ok {
            ok_count += 1;
        } else {
            fail_count += 1;
        }
    }
    log.push(String::new());
    log.push(format!(
        "summary: {ok_count}/{} green{}",
        filtered.len(),
        if fail_count > 0 {
            format!(", {fail_count} fail")
        } else {
            String::new()
        }
    ));
    true
}

#[derive(Debug, Clone)]
struct BotRow {
    bot: String,
    in_pass: bool,
    in_registry: bool,
    legacy_path: Option<PathBuf>,
    hybrid_path: Option<PathBuf>,
    tmux_line: Option<String>,
    online: bool,
    online_session: Option<String>,
    online_bun_pid: Option<u32>,
    anchor: Option<String>,
    discord_ok: Option<bool>,
    discord_status: Option<u16>,
    discord_username: Option<String>,
}

async fn status(
    env: &DiscordEnv,
    rest: &dyn DiscordRest,
    args: &[String],
    log: &mut Vec<String>,
) -> bool {
    let check = args.iter().any(|a| a == "--check");
    let redact = args.iter().any(|a| a == "--redact");
    let json_flag = args.iter().any(|a| a == "--json");
    let filter = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(String::as_str);
    let mut rows = gather_rows(env);
    if let Some(filter) = filter {
        rows.retain(|r| r.bot == filter || r.bot == format!("{filter}-oracle"));
        if rows.is_empty() {
            log.push(format!(
                "✗ no bot matching '{filter}' in pass or state-dirs.ts"
            ));
            return true;
        }
    }
    if check {
        let tokens = list_pass_tokens(env);
        for row in &mut rows {
            if let Some(entry) = tokens.iter().find(|t| t.bot == row.bot) {
                if let Some(token) = decrypt_token(&entry.name) {
                    let (ok, status, username) = ping(rest, &token).await;
                    row.discord_ok = Some(ok);
                    row.discord_status = Some(status);
                    row.discord_username = username;
                } else {
                    row.discord_ok = Some(false);
                    row.discord_status = Some(0);
                }
            }
        }
    }
    if json_flag {
        let rows_json = rows.iter().map(row_json).collect::<Vec<_>>();
        log.push(
            serde_json::to_string_pretty(
                &json!({"host": short_host(env), "redacted": redact, "rows": rows_json}),
            )
            .unwrap_or_default(),
        );
    } else if filter.is_some() && rows.len() == 1 {
        emit_status_detail(env, &rows[0], redact, log);
    } else {
        emit_status_table(env, &rows, check, redact, log);
    }
    true
}

fn gather_rows(env: &DiscordEnv) -> Vec<BotRow> {
    let tokens = list_pass_tokens(env);
    let registry = load_state_dirs_registry(env);
    let anchors = load_anchors(env);
    let all = tokens
        .iter()
        .map(|t| t.bot.clone())
        .chain(registry.iter().cloned())
        .collect::<BTreeSet<_>>();
    all.into_iter()
        .map(|bot| {
            let online = find_online_bun_for_bot(&bot);
            BotRow {
                in_pass: tokens.iter().any(|t| t.bot == bot),
                in_registry: registry.contains(&bot),
                legacy_path: find_legacy_state_dir(env, &bot),
                hybrid_path: find_hybrid_discord(env, &bot),
                tmux_line: find_tmux_session(&bot),
                online: online.is_some(),
                online_session: online.as_ref().and_then(|o| o.1.clone()),
                online_bun_pid: online.as_ref().map(|o| o.0),
                anchor: anchors.get(&bot).cloned(),
                discord_ok: None,
                discord_status: None,
                discord_username: None,
                bot,
            }
        })
        .collect()
}

fn row_json(row: &BotRow) -> Value {
    json!({
        "bot": row.bot,
        "inPass": row.in_pass,
        "inRegistry": row.in_registry,
        "legacyPath": row.legacy_path.as_ref().map(|p| p.display().to_string()),
        "hybridPath": row.hybrid_path.as_ref().map(|p| p.display().to_string()),
        "tmuxLine": row.tmux_line,
        "online": row.online,
        "onlineSession": row.online_session,
        "onlineBunPid": row.online_bun_pid,
        "anchor": row.anchor,
        "discordOK": row.discord_ok,
        "discordStatus": row.discord_status,
        "discordUsername": row.discord_username,
    })
}

fn short_host(env: &DiscordEnv) -> String {
    env.hostname
        .split('.')
        .next()
        .unwrap_or("unknown")
        .to_owned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Severity {
    Ok,
    Warn,
    Info,
    Error,
}

fn classify(row: &BotRow) -> (Severity, String) {
    if row.in_registry && !row.in_pass {
        return (
            Severity::Error,
            "registered but no token in pass".to_owned(),
        );
    }
    if row.in_pass && !row.in_registry {
        return (
            Severity::Error,
            "token in pass but not in state-dirs.ts".to_owned(),
        );
    }
    if row.discord_ok == Some(false) {
        return (
            Severity::Error,
            format!("Discord REST returned {}", row.discord_status.unwrap_or(0)),
        );
    }
    if row.tmux_line.is_some() && !row.online {
        return (
            Severity::Error,
            "tmux session exists but no Gateway bun — orphan (bind incomplete)".to_owned(),
        );
    }
    if row.in_pass && row.in_registry && row.hybrid_path.is_none() && row.online {
        return (
            Severity::Info,
            "online but using legacy state-dir — hybrid pattern not applied".to_owned(),
        );
    }
    if row.in_registry && !row.online {
        return (Severity::Warn, "offline on this host".to_owned());
    }
    (Severity::Ok, String::new())
}

fn sev_name(sev: Severity) -> &'static str {
    match sev {
        Severity::Ok => "ok",
        Severity::Warn => "warn",
        Severity::Info => "info",
        Severity::Error => "error",
    }
}

fn sev_icon(sev: Severity) -> &'static str {
    match sev {
        Severity::Ok => "✓",
        Severity::Warn => "○",
        Severity::Info => "·",
        Severity::Error => "✗",
    }
}

fn emit_status_table(
    env: &DiscordEnv,
    rows: &[BotRow],
    check: bool,
    redact: bool,
    log: &mut Vec<String>,
) {
    let host = short_host(env);
    log.push(format!(
        "🔍 maw discord status @ {host} — {} bot(s) | {}{}",
        rows.len(),
        if redact { "REDACTED · " } else { "" },
        if check {
            "with Discord REST"
        } else {
            "online/where via bun ancestry — use --check for REST"
        }
    ));
    log.push(String::new());
    let head = "  bot                          online  anchor              drift  where (tmux session)              severity";
    log.push(head.to_owned());
    log.push(format!("  {}", "─".repeat(head.len() - 2)));
    let mut counts = BTreeMap::from([
        (Severity::Ok, 0usize),
        (Severity::Warn, 0),
        (Severity::Info, 0),
        (Severity::Error, 0),
    ]);
    let mut drift_count = 0;
    for row in rows {
        let (sev, _) = classify(row);
        *counts.entry(sev).or_default() += 1;
        let is_here = row.anchor.as_ref().is_some_and(|a| {
            a == &host
                || a == &format!("nat@{host}")
                || a.ends_with(&format!("@{host}"))
                || a.ends_with(&format!("@{host}.wg"))
        });
        let drift = if row.online && row.anchor.is_some() && !is_here {
            drift_count += 1;
            "⚠ here"
        } else if row.online && row.anchor.is_some() {
            " ✓ ok "
        } else if !row.online && row.anchor.is_some() && is_here {
            "⚠ down"
        } else {
            "  ─  "
        };
        let where_text = row
            .online_session
            .clone()
            .or_else(|| row.tmux_line.as_ref().map(|_| "(orphan tmux)".to_owned()))
            .unwrap_or_else(|| "—".to_owned());
        log.push(format!(
            "  {:<28}{}  {:<18}  {drift}  {:<33} {} {}",
            row.bot,
            if row.online { "✓ ON  " } else { "✗ off " },
            row.anchor.clone().unwrap_or_else(|| "—".to_owned()),
            where_text,
            sev_icon(sev),
            sev_name(sev)
        ));
    }
    log.push(String::new());
    log.push(format!(
        "summary @ {host}: {} ok · {} warn · {} info · {} error",
        counts[&Severity::Ok],
        counts[&Severity::Warn],
        counts[&Severity::Info],
        counts[&Severity::Error]
    ));
    log.push(format!(
        "  online: {}/{}  ·  anchors: {}/{}  ·  drift: {drift_count}",
        rows.iter().filter(|r| r.online).count(),
        rows.len(),
        rows.iter().filter(|r| r.anchor.is_some()).count(),
        rows.len()
    ));
    log.push("  legend: ✓ ON = Gateway bun verified · anchor = canonical host (state-dirs.ts ANCHORS) · drift = bot online but not on anchor host".to_owned());
    if counts[&Severity::Error] > 0 || drift_count > 0 {
        log.push("run 'maw discord status <bot>' for details on any error/drift row".to_owned());
    }
}

fn emit_status_detail(env: &DiscordEnv, row: &BotRow, redact: bool, log: &mut Vec<String>) {
    let (sev, reason) = classify(row);
    let host = short_host(env);
    log.push(format!(
        "🔍 {}  @ {host}    {} {}{}",
        row.bot,
        sev_icon(sev),
        sev_name(sev),
        if reason.is_empty() {
            String::new()
        } else {
            format!(" — {reason}")
        }
    ));
    log.push(String::new());
    if row.online {
        log.push(format!("  Gateway:           ✓ ONLINE on {host}"));
        log.push(format!(
            "                       bun pid:      {}",
            row.online_bun_pid.unwrap_or(0)
        ));
        log.push(format!(
            "                       tmux session: {}",
            row.online_session
                .clone()
                .unwrap_or_else(|| "(detached)".to_owned())
        ));
    } else if let Some(tmux) = &row.tmux_line {
        log.push(
            "  Gateway:           ✗ OFFLINE — tmux session present but no Gateway bun (orphan)"
                .to_owned(),
        );
        log.push(format!("                       orphan tmux:  {tmux}"));
    } else {
        log.push(format!("  Gateway:           ✗ OFFLINE on {host}"));
    }
    if row.in_pass {
        if let Some(t) = list_pass_tokens(env).into_iter().find(|t| t.bot == row.bot) {
            let when = if redact {
                "—".to_owned()
            } else {
                t.modified.map_or_else(|| "—".to_owned(), ymd_utc)
            };
            log.push(format!(
                "  Pass token:        ✓ discord/{} ({}, {when})",
                t.name,
                fmt_size(t.size_bytes)
            ));
        }
    } else {
        log.push(format!(
            "  Pass token:        ✗ missing — no discord/{}-token in pass",
            row.bot
        ));
    }
    log.push(format!(
        "  Legacy state-dir:  {}",
        row.legacy_path.as_ref().map_or_else(
            || format!("✗ not found at ~/.claude/channels/{}/", row.bot),
            |p| format!("✓ {}/", p.display())
        )
    ));
    log.push(format!(
        "  Hybrid .discord/:  {}",
        row.hybrid_path.as_ref().map_or_else(
            || "✗ not found".to_owned(),
            |p| format!("✓ {}/", p.display())
        )
    ));
    log.push(format!(
        "  Registry:          {}",
        if row.in_registry {
            "✓ in state-dirs.ts"
        } else {
            "✗ missing from state-dirs.ts"
        }
    ));
    log.push(format!(
        "  Anchor:            {}",
        row.anchor.clone().unwrap_or_else(|| "—".to_owned())
    ));
    if let Some(ok) = row.discord_ok {
        let username = row
            .discord_username
            .clone()
            .unwrap_or_else(|| "—".to_owned());
        log.push(format!(
            "  Discord REST:      {} {}  {username}",
            if ok { "✓" } else { "✗" },
            row.discord_status.unwrap_or(0)
        ));
    } else {
        log.push("  Discord REST:      (not checked — add --check)".to_owned());
    }
}

fn bind(env: &DiscordEnv, args: &[String], log: &mut Vec<String>) -> bool {
    let Some(bot) = args.first() else {
        log.push(
            "usage: maw discord bind <bot> [--apply] [--restart] [--session <name>] [--force]"
                .to_owned(),
        );
        log.push(String::new());
        log.push("  --apply      execute the plan (default: dry-run)".to_owned());
        log.push(
            "  --restart    if already online, telegraph + kill the existing session first"
                .to_owned(),
        );
        log.push("  --session    custom tmux session name (default: <bot>-discord)".to_owned());
        log.push(
            "  --force      override 'attached clients' check on --restart (yanks panes)"
                .to_owned(),
        );
        return true;
    };
    if rejects_option_arg(bot) {
        log.push("✗ invalid bot name: leading dash/-- separator rejected".to_owned());
        return true;
    }
    let apply = args.iter().any(|a| a == "--apply");
    let session = flag_value(args, "--session").unwrap_or_else(|| format!("{bot}-discord"));
    log.push(format!(
        "🪣 maw discord bind {bot}{}",
        if apply {
            " --apply"
        } else {
            " (dry-run — pass --apply to execute)"
        }
    ));
    log.push(String::new());
    let token = list_pass_tokens(env).into_iter().find(|t| t.bot == *bot);
    let state_dir = find_hybrid_discord(env, bot).or_else(|| find_legacy_state_dir(env, bot));
    let online = find_online_bun_for_bot(bot);
    log.push("  pre-flight:".to_owned());
    log.push(format!(
        "    {} pass token            {}",
        if token.is_some() { "✓" } else { "✗" },
        token.as_ref().map_or_else(
            || format!("missing discord/{bot}-token"),
            |t| format!("discord/{}", t.name)
        )
    ));
    log.push(format!(
        "    {} state-dir             {}",
        if state_dir.is_some() { "✓" } else { "✗" },
        state_dir.as_ref().map_or_else(
            || "missing hybrid .discord or legacy ~/.claude/channels".to_owned(),
            |p| p.display().to_string()
        )
    ));
    log.push(format!(
        "    {} not already online    {}",
        if online.is_none() { "✓" } else { "✗" },
        online.as_ref().map_or_else(
            || "ok".to_owned(),
            |o| format!(
                "already online pid {} tmux {}",
                o.0,
                o.1.clone().unwrap_or_else(|| "?".to_owned())
            )
        )
    ));
    log.push(String::new());
    if token.is_none() || state_dir.is_none() || online.is_some() {
        log.push("  ✗ pre-flight failed. fix the failing checks above and re-run.".to_owned());
        if online.is_some() {
            log.push("     to restart anyway, re-run with --restart (telegraphs + kills the existing session)".to_owned());
        }
        return true;
    }
    let cwd = find_ghq_path(env, bot).unwrap_or_else(|| env.ghq_root.join(bot));
    log.push("  plan:".to_owned());
    log.push(format!("    session: {session}"));
    log.push(format!("    cwd:     {}", cwd.display()));
    log.push(format!(
        "    state:   {}",
        state_dir.expect("checked").display()
    ));
    log.push("    command: claude --channels plugin:discord@claude-plugins-official".to_owned());
    log.push(String::new());
    if !apply {
        log.push("  ⓘ dry-run only — re-run with --apply to execute".to_owned());
        return true;
    }
    log.push(
        "  ✗ native maw-rs bind apply is intentionally not implemented in REST-only piece 1"
            .to_owned(),
    );
    log.push(
        "    use dry-run output for review; no gateway/websocket process is launched here"
            .to_owned(),
    );
    true
}

async fn access(
    env: &DiscordEnv,
    rest: &dyn DiscordRest,
    args: &[String],
    log: &mut Vec<String>,
) -> bool {
    if args.is_empty() {
        access_usage(log);
        return true;
    }
    let bot = &args[0];
    let sub = args.get(1).map_or("", String::as_str).to_lowercase();
    if sub.is_empty() || matches!(sub.as_str(), "help" | "-h" | "--help") {
        access_usage(log);
        return true;
    }
    let Some(pre) = resolve_bot(env, bot, log) else {
        return true;
    };
    log.push(format!(
        "🪪 maw discord access {bot} {sub}{}",
        if args.len() > 2 {
            format!(" {}", args[2..].join(" "))
        } else {
            String::new()
        }
    ));
    log.push(format!(
        "  state-dir: {}{}",
        pre.state_dir.display(),
        if pre.is_hybrid {
            " (hybrid)"
        } else {
            " (legacy)"
        }
    ));
    log.push(String::new());
    match sub.as_str() {
        "list" => access_list(&pre, &args[2..], log),
        "show" => access_show(&pre, &args[2..], log),
        "map" => access_map(&pre, rest, &args[2..], log).await,
        "add" => access_add(&pre, &args[2..], log),
        "rm" => access_rm(&pre, &args[2..], log),
        "set" => access_set(&pre, &args[2..], log),
        "allow" => access_allow(&pre, &args[2..], log),
        "lockdown" => access_lockdown(&pre, &args[2..], log),
        _ => {
            log.push(format!("✗ unknown subcommand: {sub}"));
            access_usage(log);
            true
        }
    }
}

fn access_usage(log: &mut Vec<String>) {
    log.extend([
        "usage: maw discord access <bot> <subcommand> [args]".to_owned(),
        String::new(),
        "subcommands:".to_owned(),
        "  list [--json]                       enabled channels for <bot>".to_owned(),
        "  show <channel> [--json]             inspect one channel's config".to_owned(),
        "  map [--guild <id>] [--refresh]      channel-map (name → id), --refresh from Discord"
            .to_owned(),
        "  add <channel> [--no-mention] [--allow <id>...]".to_owned(),
        "                                      enable channel access".to_owned(),
        "  rm <channel> [--dry-run]            remove channel access".to_owned(),
        "  set <channel> [--no-mention|--mention] [--allow <id>...]".to_owned(),
        "                                      toggle existing channel without rm+add".to_owned(),
        "  allow <add|rm|ls> [<user-id>]       global DM allowlist management".to_owned(),
        "  lockdown [--off] [--dry-run]        dmPolicy=allowlist (or revert with --off)"
            .to_owned(),
    ]);
}

#[derive(Debug, Clone)]
struct BotResolved {
    bot: String,
    state_dir: PathBuf,
    token_name: String,
    is_hybrid: bool,
    access_json: PathBuf,
    channel_map: PathBuf,
}

fn resolve_bot(env: &DiscordEnv, bot: &str, log: &mut Vec<String>) -> Option<BotResolved> {
    if rejects_option_arg(bot) {
        log.push("✗ invalid bot name: leading dash/-- separator rejected".to_owned());
        return None;
    }
    let registry = load_state_dirs_registry(env);
    let hybrid = find_hybrid_discord(env, bot);
    let legacy = find_legacy_state_dir(env, bot);
    let state_dir = match hybrid.clone().or(legacy) {
        Some(path) => path,
        None => {
            log.push(format!("✗ no state-dir found for '{bot}' (checked hybrid <repo>/.discord/ and {}/.claude/channels/{bot}/)", env.home.display()));
            return None;
        }
    };
    let Some(tok) = list_pass_tokens(env).into_iter().find(|t| t.bot == bot) else {
        log.push(format!(
            "✗ no pass entry for '{bot}' (looked for discord/{bot}-token.gpg)"
        ));
        return None;
    };
    if !registry.contains(bot) {
        log.push(format!(
            "⚠ '{bot}' not in discord-oracle/src/state-dirs.ts — dashboard won't see it"
        ));
    }
    Some(BotResolved {
        bot: bot.to_owned(),
        token_name: tok.name,
        is_hybrid: hybrid.is_some(),
        access_json: state_dir.join("access.json"),
        channel_map: state_dir.join("channel-map.json"),
        state_dir,
    })
}

fn load_access(path: &Path) -> AccessFile {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_access(path: &Path, access: &AccessFile) -> Result<(), String> {
    let body = serde_json::to_string_pretty(access).map_err(|e| e.to_string())? + "\n";
    fs::write(path, body).map_err(|e| e.to_string())
}

fn load_channel_map(path: &Path) -> BTreeMap<String, String> {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_channel_map(path: &Path, map: &BTreeMap<String, String>) -> Result<(), String> {
    let body = serde_json::to_string_pretty(map).map_err(|e| e.to_string())? + "\n";
    fs::write(path, body).map_err(|e| e.to_string())
}

fn resolve_channel(map: &BTreeMap<String, String>, name: &str) -> Option<String> {
    if name.chars().all(|c| c.is_ascii_digit()) {
        Some(name.to_owned())
    } else {
        map.get(name).cloned()
    }
}

fn parse_flags(args: &[String]) -> (Vec<String>, HashMap<String, Vec<String>>) {
    let mut pos = Vec::new();
    let mut flags: HashMap<String, Vec<String>> = HashMap::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--no-mention" | "--mention" | "--dry-run" | "--json" | "--refresh" | "--off"
            | "--all-guilds" | "--with-threads" => {
                flags
                    .entry(args[i].trim_start_matches("--").to_owned())
                    .or_default()
                    .push("true".to_owned());
            }
            "--guild" | "--allow" => {
                if let Some(v) = args.get(i + 1) {
                    flags
                        .entry(args[i].trim_start_matches("--").to_owned())
                        .or_default()
                        .push(v.clone());
                    i += 1;
                }
            }
            a => pos.push(a.to_owned()),
        }
        i += 1;
    }
    (pos, flags)
}

fn has_flag(flags: &HashMap<String, Vec<String>>, name: &str) -> bool {
    flags.contains_key(name)
}

fn access_list(pre: &BotResolved, args: &[String], log: &mut Vec<String>) -> bool {
    let (_, flags) = parse_flags(args);
    let access = load_access(&pre.access_json);
    let map = load_channel_map(&pre.channel_map);
    let reverse = reverse_map(&map);
    let entries = access
        .groups
        .iter()
        .map(|(id, cfg)| {
            (
                id,
                reverse
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| "(unknown)".to_owned()),
                cfg,
            )
        })
        .collect::<Vec<_>>();
    if has_flag(&flags, "json") {
        log.push(serde_json::to_string_pretty(&json!({"bot": pre.bot, "channels": entries.iter().map(|(id, name, cfg)| json!({"id": id, "name": name, "requireMention": cfg.require_mention, "allowFrom": cfg.allow_from})).collect::<Vec<_>>() })).unwrap_or_default());
        return true;
    }
    if entries.is_empty() {
        log.push("  (no channels enabled)".to_owned());
        return true;
    }
    log.push(format!("  {} channel(s):", entries.len()));
    log.push(String::new());
    log.push(
        "  channel-name                     id                    mention  allowFrom".to_owned(),
    );
    log.push(
        "  ─────────────────────────────────────────────────────────────────────────".to_owned(),
    );
    for (id, name, cfg) in entries {
        let mention = if cfg.require_mention {
            "✓ tag  "
        } else {
            "○ all  "
        };
        log.push(format!(
            "  {:<32} {:<20}  {mention}  {}",
            name,
            id,
            if cfg.allow_from.is_empty() {
                "(none)".to_owned()
            } else {
                cfg.allow_from.join(",")
            }
        ));
    }
    true
}

fn access_show(pre: &BotResolved, args: &[String], log: &mut Vec<String>) -> bool {
    let (pos, flags) = parse_flags(args);
    let Some(channel_arg) = pos.first() else {
        log.push("usage: maw discord access <bot> show <channel> [--json]".to_owned());
        return true;
    };
    let map = load_channel_map(&pre.channel_map);
    let Some(id) = resolve_channel(&map, channel_arg) else {
        log.push(format!(
            "✗ channel '{channel_arg}' not in channel-map (run 'access map --refresh')"
        ));
        return true;
    };
    let access = load_access(&pre.access_json);
    let Some(cfg) = access.groups.get(&id) else {
        log.push(format!(
            "✗ channel '{channel_arg}' ({id}) not in access.json"
        ));
        return true;
    };
    let reverse = reverse_map(&map);
    let name = reverse
        .get(&id)
        .cloned()
        .unwrap_or_else(|| "(unknown)".to_owned());
    if has_flag(&flags, "json") {
        log.push(serde_json::to_string_pretty(&json!({"bot": pre.bot, "channel": {"id": id, "name": name}, "requireMention": cfg.require_mention, "allowFrom": cfg.allow_from })).unwrap_or_default());
        return true;
    }
    log.push(format!("  #{name} ({id})"));
    log.push(format!("    requireMention: {}", cfg.require_mention));
    log.push(format!(
        "    allowFrom:      {}",
        if cfg.allow_from.is_empty() {
            "(none)".to_owned()
        } else {
            cfg.allow_from.join(", ")
        }
    ));
    true
}

async fn access_map(
    pre: &BotResolved,
    rest: &dyn DiscordRest,
    args: &[String],
    log: &mut Vec<String>,
) -> bool {
    let (_, flags) = parse_flags(args);
    if has_flag(&flags, "refresh") {
        log.push("  refreshing channel-map from Discord...".to_owned());
        if let Some(token) = decrypt_token(&pre.token_name) {
            let guilds = fetch_guilds(rest, &token).await.unwrap_or_default();
            let mut map = load_channel_map(&pre.channel_map);
            let guild_filter = flags.get("guild").and_then(|v| v.first());
            for guild in guilds
                .iter()
                .filter(|g| guild_filter.is_none_or(|id| id == &g.id))
            {
                if let Ok(channels) = fetch_channels(rest, &token, &guild.id).await {
                    for channel in channels {
                        if channel.kind == 0 || channel.kind == 5 {
                            map.insert(channel.name, channel.id);
                        }
                    }
                }
            }
            if let Err(error) = save_channel_map(&pre.channel_map, &map) {
                log.push(format!("  ✗ failed to write channel-map: {error}"));
            } else {
                log.push(format!("    wrote {} channel(s)", map.len()));
            }
            log.push(String::new());
        } else {
            log.push(format!("  ✗ pass decrypt failed for {}", pre.token_name));
        }
    }
    let map = load_channel_map(&pre.channel_map);
    if map.is_empty() {
        log.push("  (no channels mapped — run with --refresh --guild <id>)".to_owned());
        return true;
    }
    log.push(format!("  {} channel(s) in map:", map.len()));
    log.push(String::new());
    log.push("  channel-name                     id".to_owned());
    log.push("  ──────────────────────────────────────────────".to_owned());
    for (name, id) in map {
        log.push(format!("  {:<32} {id}", name));
    }
    true
}

fn access_add(pre: &BotResolved, args: &[String], log: &mut Vec<String>) -> bool {
    let (pos, flags) = parse_flags(args);
    let Some(channel_arg) = pos.first() else {
        log.push(
            "usage: maw discord access <bot> add <channel> [--no-mention] [--allow <id>...]"
                .to_owned(),
        );
        return true;
    };
    let map = load_channel_map(&pre.channel_map);
    let Some(id) = resolve_channel(&map, channel_arg) else {
        log.push(format!("✗ channel '{channel_arg}' not in channel-map"));
        log.push(format!(
            "  run 'maw discord access {} map --refresh' first",
            pre.bot
        ));
        return true;
    };
    let mut access = load_access(&pre.access_json);
    let allow = flags.get("allow").cloned().unwrap_or_default();
    access.groups.insert(
        id.clone(),
        AccessGroup {
            require_mention: !has_flag(&flags, "no-mention"),
            allow_from: allow.clone(),
        },
    );
    if let Err(error) = save_access(&pre.access_json, &access) {
        log.push(format!("✗ failed to save access.json: {error}"));
        return true;
    }
    log.push(format!("  ✓ enabled #{channel_arg} ({id})"));
    if has_flag(&flags, "no-mention") || !allow.is_empty() {
        log.push(format!(
            "  ✓ flags applied: {}{}",
            if has_flag(&flags, "no-mention") {
                "mention=false "
            } else {
                ""
            },
            if allow.is_empty() {
                String::new()
            } else {
                format!("allow=[{}]", allow.join(","))
            }
        ));
    } else {
        log.push(
            "  (defaults applied: requireMention=true, allowFrom=[$DISCORD_USER_ID])".to_owned(),
        );
    }
    true
}

fn access_rm(pre: &BotResolved, args: &[String], log: &mut Vec<String>) -> bool {
    let (pos, flags) = parse_flags(args);
    let Some(channel_arg) = pos.first() else {
        log.push("usage: maw discord access <bot> rm <channel> [--dry-run]".to_owned());
        return true;
    };
    let map = load_channel_map(&pre.channel_map);
    let Some(id) = resolve_channel(&map, channel_arg) else {
        log.push(format!("✗ channel '{channel_arg}' not in channel-map"));
        return true;
    };
    let mut access = load_access(&pre.access_json);
    if !access.groups.contains_key(&id) {
        log.push(format!("✗ channel '{channel_arg}' not currently enabled"));
        return true;
    }
    if has_flag(&flags, "dry-run") {
        log.push(format!(
            "  [dry-run] would remove #{channel_arg} ({id}) from access"
        ));
        log.push(format!(
            "            current config: {}",
            serde_json::to_string(&access.groups[&id]).unwrap_or_default()
        ));
        return true;
    }
    access.groups.remove(&id);
    if let Err(error) = save_access(&pre.access_json, &access) {
        log.push(format!("✗ failed to save access.json: {error}"));
    } else {
        log.push(format!("  ✓ removed #{channel_arg} ({id})"));
    }
    true
}

fn access_set(pre: &BotResolved, args: &[String], log: &mut Vec<String>) -> bool {
    let (pos, flags) = parse_flags(args);
    let Some(channel_arg) = pos.first() else {
        log.push("usage: maw discord access <bot> set <channel> [--no-mention|--mention] [--allow <id>...]".to_owned());
        return true;
    };
    let map = load_channel_map(&pre.channel_map);
    let Some(id) = resolve_channel(&map, channel_arg) else {
        log.push(format!("✗ channel '{channel_arg}' not in channel-map"));
        return true;
    };
    let mut access = load_access(&pre.access_json);
    let Some(cfg) = access.groups.get_mut(&id) else {
        log.push("✗ channel not currently enabled — use 'add' instead".to_owned());
        return true;
    };
    if has_flag(&flags, "no-mention") {
        cfg.require_mention = false;
    }
    if has_flag(&flags, "mention") {
        cfg.require_mention = true;
    }
    if let Some(allow) = flags.get("allow") {
        cfg.allow_from = allow.clone();
    }
    if let Err(error) = save_access(&pre.access_json, &access) {
        log.push(format!("✗ failed to save access.json: {error}"));
    } else if let Some(cfg) = access.groups.get(&id) {
        log.push(format!(
            "  ✓ updated: {}allow=[{}]",
            if cfg.require_mention {
                "mention=true "
            } else {
                "mention=false "
            },
            cfg.allow_from.join(",")
        ));
    }
    true
}

fn access_allow(pre: &BotResolved, args: &[String], log: &mut Vec<String>) -> bool {
    let action = args.first().map_or("", String::as_str);
    if !matches!(action, "add" | "rm" | "ls") {
        log.push("usage: maw discord access <bot> allow <add|rm|ls> [<user-id>]".to_owned());
        return true;
    }
    let mut access = load_access(&pre.access_json);
    if action == "ls" {
        log.push(format!("  global allowlist ({}):", access.allow_from.len()));
        for id in access.allow_from {
            log.push(format!("    {id}"));
        }
        return true;
    }
    let Some(user_id) = args.get(1) else {
        log.push(format!(
            "usage: maw discord access <bot> allow {action} <user-id>"
        ));
        return true;
    };
    if action == "add" {
        if access.allow_from.contains(user_id) {
            log.push(format!("  ○ {user_id} already in allowlist"));
        } else {
            access.allow_from.push(user_id.clone());
            let _ = save_access(&pre.access_json, &access);
            log.push(format!("  ✓ added {user_id} to global allowlist"));
        }
    } else if let Some(index) = access.allow_from.iter().position(|id| id == user_id) {
        access.allow_from.remove(index);
        let _ = save_access(&pre.access_json, &access);
        log.push(format!("  ✓ removed {user_id} from global allowlist"));
    } else {
        log.push(format!("  ○ {user_id} not in allowlist"));
    }
    true
}

fn access_lockdown(pre: &BotResolved, args: &[String], log: &mut Vec<String>) -> bool {
    let (_, flags) = parse_flags(args);
    let mut access = load_access(&pre.access_json);
    let target = if has_flag(&flags, "off") {
        "open"
    } else {
        "allowlist"
    };
    let current = access.dm_policy.clone();
    if current == target {
        log.push(format!("  ○ dmPolicy already '{target}' — no change"));
        return true;
    }
    if has_flag(&flags, "dry-run") {
        log.push(format!(
            "  [dry-run] would set dmPolicy: '{current}' → '{target}'"
        ));
        return true;
    }
    access.dm_policy = target.to_owned();
    let _ = save_access(&pre.access_json, &access);
    log.push(format!("  ✓ dmPolicy: '{current}' → '{target}'"));
    true
}

async fn guilds(
    env: &DiscordEnv,
    rest: &dyn DiscordRest,
    args: &[String],
    log: &mut Vec<String>,
) -> bool {
    let Some(bot) = args.first() else {
        log.push("usage: maw discord guilds <bot> [--json]".to_owned());
        return true;
    };
    let Some(pre) = resolve_bot_for_rest(env, bot, log) else {
        return true;
    };
    let json_flag = args.iter().any(|a| a == "--json");
    let Ok(guilds) = fetch_guilds(rest, &pre.1).await else {
        log.push("✗ guilds REST failed".to_owned());
        return true;
    };
    if json_flag {
        log.push(
            serde_json::to_string_pretty(&json!({"bot": bot, "guilds": guilds}))
                .unwrap_or_default(),
        );
        return true;
    }
    log.push(format!("🌐 {bot} is in {} server(s):", guilds.len()));
    log.push(String::new());
    log.push("  id                    name".to_owned());
    log.push("  ────────────────────  ────────────────────────────────────".to_owned());
    for guild in guilds {
        log.push(format!("  {}  {}", guild.id, guild.name));
    }
    true
}

async fn channels(
    env: &DiscordEnv,
    rest: &dyn DiscordRest,
    args: &[String],
    log: &mut Vec<String>,
) -> bool {
    let Some(bot) = args.first() else {
        log.push("usage: maw discord channels <bot> [--guild <id>] [--all-guilds] [--json] [--with-threads]".to_owned());
        return true;
    };
    let Some((_, token, _)) = resolve_bot_for_rest(env, bot, log) else {
        return true;
    };
    let (_, flags) = parse_flags(&args[1..]);
    let guilds = fetch_guilds(rest, &token).await.unwrap_or_default();
    let targets = flags
        .get("guild")
        .and_then(|v| v.first())
        .map_or(guilds.clone(), |id| {
            guilds.into_iter().filter(|g| &g.id == id).collect()
        });
    let mut out = Vec::new();
    for guild in targets {
        match fetch_channels(rest, &token, &guild.id).await {
            Ok(chs) => out.push((guild, chs)),
            Err(e) => log.push(format!("  ⚠ {} {}: {e}", guild.id, guild.name)),
        }
    }
    if has_flag(&flags, "json") {
        log.push(serde_json::to_string_pretty(&json!({"bot": bot, "guilds": out.iter().map(|(g, c)| json!({"guild": g, "channels": c})).collect::<Vec<_>>() })).unwrap_or_default());
        return true;
    }
    log.push(format!("📺 {bot} channels across {} guild(s):", out.len()));
    log.push(String::new());
    for (guild, chs) in out {
        log.push(format!(
            "  ▼ {} ({})  ·  {} channel(s)",
            guild.name,
            guild.id,
            chs.len()
        ));
        for c in chs
            .iter()
            .filter(|c| has_flag(&flags, "with-threads") || !matches!(c.kind, 10..=12))
        {
            log.push(format!(
                "     {}  {:<6}  #{:<36} {}",
                c.id,
                channel_type_label(c.kind),
                c.name,
                c.parent_id.clone().unwrap_or_default()
            ));
        }
        log.push(String::new());
    }
    true
}

async fn members(
    env: &DiscordEnv,
    rest: &dyn DiscordRest,
    args: &[String],
    log: &mut Vec<String>,
) -> bool {
    let (Some(bot), Some(channel_arg)) = (args.first(), args.get(1)) else {
        log.push("usage: maw discord members <bot> <channel-name-or-id> [--json]".to_owned());
        return true;
    };
    let Some((pre, token, _)) = resolve_bot_for_rest(env, bot, log) else {
        return true;
    };
    let map = load_channel_map(&pre.channel_map);
    let Some(channel_id) = resolve_channel(&map, channel_arg) else {
        log.push(format!("✗ channel '{channel_arg}' not in channel-map. Run 'maw discord access {bot} map --guild <id> --refresh'"));
        return true;
    };
    let access = load_access(&pre.access_json);
    let Some(cfg) = access.groups.get(&channel_id) else {
        log.push(format!(
            "✗ channel {channel_id} not in access.json groups for {bot}"
        ));
        return true;
    };
    let pairs = resolve_user_list(rest, &token, &cfg.allow_from).await;
    let result = json!({"bot": bot, "channelId": channel_id, "requireMention": cfg.require_mention, "allowFrom": pairs, "effective": if cfg.allow_from.is_empty() {"mention-only"} else {"allowlist"}});
    if args.iter().any(|a| a == "--json") {
        log.push(serde_json::to_string_pretty(&result).unwrap_or_default());
        return true;
    }
    log.push(format!("👥 {bot} · #{channel_arg} ({channel_id})"));
    log.push(format!("   requireMention: {}", cfg.require_mention));
    if cfg.allow_from.is_empty() {
        log.push("   allowFrom:      (none)".to_owned());
    } else {
        log.push("   allowFrom:".to_owned());
        for pair in result["allowFrom"].as_array().into_iter().flatten() {
            log.push(format!(
                "     · {:<18} ({})",
                pair["name"].as_str().unwrap_or_default(),
                pair["id"].as_str().unwrap_or_default()
            ));
        }
    }
    log.push(format!(
        "   effective:      {}",
        result["effective"].as_str().unwrap_or_default()
    ));
    true
}

async fn inventory(
    env: &DiscordEnv,
    rest: &dyn DiscordRest,
    args: &[String],
    log: &mut Vec<String>,
) -> bool {
    let Some(bot) = args.first() else {
        log.push("usage: maw discord inventory <bot> [--json]".to_owned());
        return true;
    };
    let Some((pre, token, _)) = resolve_bot_for_rest(env, bot, log) else {
        return true;
    };
    let guilds = fetch_guilds(rest, &token).await.unwrap_or_default();
    let access = load_access(&pre.access_json);
    let mut rows = Vec::new();
    let mut total_channels = 0usize;
    let mut total_enabled = 0usize;
    for guild in guilds {
        match fetch_channels(rest, &token, &guild.id).await {
            Ok(chs) => {
                total_channels += chs.len();
                total_enabled += chs
                    .iter()
                    .filter(|c| access.groups.contains_key(&c.id))
                    .count();
                rows.push((guild, chs));
            }
            Err(e) => log.push(format!("  ⚠ {} {}: {e}", guild.id, guild.name)),
        }
    }
    if args.iter().any(|a| a == "--json") {
        log.push(serde_json::to_string_pretty(&json!({"bot": bot, "inventory": rows.iter().map(|(g, c)| json!({"guild": g, "channels": c})).collect::<Vec<_>>() })).unwrap_or_default());
        return true;
    }
    let all_ids = access
        .groups
        .values()
        .flat_map(|g| g.allow_from.clone())
        .collect::<BTreeSet<_>>();
    let names = resolve_user_list(rest, &token, &all_ids.into_iter().collect::<Vec<_>>()).await;
    let name_by_id = names
        .into_iter()
        .filter_map(|v| {
            Some((
                v.get("id")?.as_str()?.to_owned(),
                v.get("name")?.as_str()?.to_owned(),
            ))
        })
        .collect::<HashMap<_, _>>();
    log.push(format!("📋 {bot} — full inventory"));
    log.push(String::new());
    for (guild, channels) in rows {
        let enabled = channels
            .iter()
            .filter(|c| access.groups.contains_key(&c.id))
            .count();
        log.push(format!(
            "  ▼ {}  ({})  ·  {enabled}/{} enabled",
            guild.name,
            guild.id,
            channels.len()
        ));
        for channel in channels {
            if let Some(cfg) = access.groups.get(&channel.id) {
                let mention = if cfg.require_mention {
                    "mention"
                } else {
                    "all-msg"
                };
                let allow = if cfg.allow_from.is_empty() {
                    "(none)".to_owned()
                } else {
                    cfg.allow_from
                        .iter()
                        .map(|id| name_by_id.get(id).cloned().unwrap_or_else(|| id.clone()))
                        .collect::<Vec<_>>()
                        .join(",")
                };
                log.push(format!("     ✓ #{:<36} {mention} {allow}", channel.name));
            } else {
                log.push(format!(
                    "     · #{:<36} (in guild, no access)",
                    channel.name
                ));
            }
        }
        log.push(String::new());
    }
    log.push(format!("summary: {} server(s) · {total_channels} channels visible · {total_enabled} enabled · {} unique allow-users resolved", fetch_guilds(rest, &token).await.unwrap_or_default().len(), name_by_id.len()));
    true
}

fn resolve_bot_for_rest(
    env: &DiscordEnv,
    bot: &str,
    log: &mut Vec<String>,
) -> Option<(BotResolved, String, String)> {
    let pre = resolve_bot(env, bot, log)?;
    let token = decrypt_token(&pre.token_name)?;
    Some((pre, token, bot.to_owned()))
}

async fn fetch_guilds(rest: &dyn DiscordRest, token: &str) -> Result<Vec<Guild>, String> {
    let res = rest.get_json("/users/@me/guilds", token).await?;
    if !(200..300).contains(&res.status) {
        return Err(format!("guilds REST {}", res.status));
    }
    serde_json::from_value(res.body).map_err(|e| e.to_string())
}

async fn fetch_channels(
    rest: &dyn DiscordRest,
    token: &str,
    guild_id: &str,
) -> Result<Vec<Channel>, String> {
    if !is_numeric_snowflake(guild_id) {
        return Err(format!("invalid guild id '{guild_id}'"));
    }
    let res = rest
        .get_json(&format!("/guilds/{guild_id}/channels"), token)
        .await?;
    if !(200..300).contains(&res.status) {
        return Err(format!("channels REST {} for guild {guild_id}", res.status));
    }
    serde_json::from_value(res.body).map_err(|e| e.to_string())
}

async fn resolve_user_list(rest: &dyn DiscordRest, token: &str, ids: &[String]) -> Vec<Value> {
    let mut out = Vec::new();
    for id in ids {
        if !is_numeric_snowflake(id) {
            out.push(json!({"id": id, "name": id, "invalid": true}));
            continue;
        }
        let path = format!("/users/{id}");
        let name = match rest.get_json(&path, token).await {
            Ok(res) if (200..300).contains(&res.status) => res
                .body
                .get("global_name")
                .or_else(|| res.body.get("username"))
                .and_then(Value::as_str)
                .map_or_else(|| id.clone(), ToOwned::to_owned),
            _ => id.clone(),
        };
        out.push(json!({"id": id, "name": name}));
    }
    out
}

fn is_numeric_snowflake(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_digit())
}

fn channel_type_label(kind: u8) -> String {
    match kind {
        0 => "text".to_owned(),
        2 => "voice".to_owned(),
        4 => "cat".to_owned(),
        5 => "news".to_owned(),
        10..=12 => "thread".to_owned(),
        13 => "stage".to_owned(),
        15 => "forum".to_owned(),
        16 => "media".to_owned(),
        _ => format!("t{kind}"),
    }
}

fn reverse_map(map: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    map.iter().map(|(k, v)| (v.clone(), k.clone())).collect()
}

fn find_legacy_state_dir(env: &DiscordEnv, bot: &str) -> Option<PathBuf> {
    let path = env.legacy_state_root().join(bot);
    path.exists().then_some(path)
}

fn find_hybrid_discord(env: &DiscordEnv, bot: &str) -> Option<PathBuf> {
    let path = find_ghq_path(env, bot)?.join(".discord");
    path.exists().then_some(path)
}

fn find_ghq_path(env: &DiscordEnv, name: &str) -> Option<PathBuf> {
    if rejects_option_arg(name) {
        return None;
    }
    let mut found = Vec::new();
    collect_dirs_named(&env.ghq_root, name, 0, &mut found);
    found.sort();
    found
        .iter()
        .find(|p| p.to_string_lossy().contains("/Soul-Brews-Studio/"))
        .cloned()
        .or_else(|| found.into_iter().next())
}

fn collect_dirs_named(root: &Path, name: &str, depth: usize, found: &mut Vec<PathBuf>) {
    if depth > 5 || found.len() > 32 {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.file_name().and_then(|s| s.to_str()) == Some(name) {
            found.push(path.clone());
        }
        collect_dirs_named(&path, name, depth + 1, found);
    }
}

fn load_state_dirs_registry(env: &DiscordEnv) -> BTreeSet<String> {
    let Some(repo) = find_ghq_path(env, "discord-oracle") else {
        return BTreeSet::new();
    };
    let Ok(content) = fs::read_to_string(repo.join("src/state-dirs.ts")) else {
        return BTreeSet::new();
    };
    let state_block = content
        .split("export const ANCHORS")
        .next()
        .unwrap_or_default();
    quoted_keys(state_block).into_iter().collect()
}

fn load_anchors(env: &DiscordEnv) -> BTreeMap<String, String> {
    let Some(repo) = find_ghq_path(env, "discord-oracle") else {
        return BTreeMap::new();
    };
    let Ok(content) = fs::read_to_string(repo.join("src/state-dirs.ts")) else {
        return BTreeMap::new();
    };
    let Some(block) = content.split("export const ANCHORS").nth(1) else {
        return BTreeMap::new();
    };
    quoted_pairs(block).into_iter().collect()
}

fn quoted_keys(input: &str) -> Vec<String> {
    input
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix('"')?;
            let (key, after) = rest.split_once('"')?;
            after.trim_start().starts_with(':').then(|| key.to_owned())
        })
        .collect()
}

fn quoted_pairs(input: &str) -> Vec<(String, String)> {
    input
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix('"')?;
            let (key, after_key) = rest.split_once('"')?;
            let value_start = after_key.split_once('"')?.1;
            let (value, _) = value_start.split_once('"')?;
            Some((key.to_owned(), value.to_owned()))
        })
        .collect()
}

fn find_tmux_session(bot: &str) -> Option<String> {
    let out = Command::new("tmux").arg("ls").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .find(|line| line.contains(bot))
        .map(ToOwned::to_owned)
}

fn find_online_bun_for_bot(bot: &str) -> Option<(u32, Option<String>)> {
    let out = Command::new("pgrep")
        .args(["-f", "discord/0.0.4"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    for pid in text.lines().filter_map(|s| s.trim().parse::<u32>().ok()) {
        let env_out = Command::new("ps")
            .args(["Eww", "-p", &pid.to_string()])
            .output()
            .ok()?;
        let env_text = String::from_utf8_lossy(&env_out.stdout);
        if env_text.contains("DISCORD_STATE_DIR=") && env_text.contains(&format!("/{bot}")) {
            return Some((
                pid,
                find_tmux_session(bot)
                    .map(|line| line.split(':').next().unwrap_or(&line).to_owned()),
            ));
        }
    }
    None
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find_map(|w| (w[0] == flag).then(|| w[1].clone()))
}

fn fmt_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes}B")
    } else {
        format!("{:.1}K", bytes as f64 / 1024.0)
    }
}

fn ymd_utc(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

fn civil_from_days(days_since_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + i64::from(m <= 2);
    (
        i32::try_from(year).unwrap_or(1970),
        u32::try_from(m).unwrap_or(1),
        u32::try_from(d).unwrap_or(1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct MockRest {
        calls: Mutex<Vec<String>>,
        responses: BTreeMap<String, DiscordHttpResponse>,
    }

    impl MockRest {
        fn new(responses: BTreeMap<String, DiscordHttpResponse>) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                responses,
            }
        }
    }

    impl DiscordRest for MockRest {
        fn get_json<'a>(
            &'a self,
            path: &'a str,
            _token: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<DiscordHttpResponse, String>> + Send + 'a>>
        {
            Box::pin(async move {
                assert!(path.starts_with('/'));
                assert!(!path.contains("://"));
                self.calls.lock().expect("calls").push(path.to_owned());
                self.responses
                    .get(path)
                    .cloned()
                    .ok_or_else(|| format!("missing mock {path}"))
            })
        }
    }

    #[tokio::test]
    async fn version_matches_maw_js_surface() {
        let env = DiscordEnv {
            home: PathBuf::from("/tmp/none"),
            ghq_root: PathBuf::from("/tmp/none"),
            hostname: "host.test".to_owned(),
        };
        let rest = MockRest::new(BTreeMap::new());
        let out = run_discord_command_with(&["version".to_owned()], &env, &rest).await;
        assert_eq!(out.code, 0);
        assert!(out.stdout.contains("maw discord v0.4.2"));
        assert!(out
            .stdout
            .contains("✓ guilds/channels/members/inventory <bot>  v0.4.2"));
    }

    #[test]
    fn reqwest_client_rejects_non_host_relative_paths() {
        let client = ReqwestDiscordRest::new().expect("client");
        assert_eq!(
            client.url_for("/users/@me").expect("url"),
            "https://discord.com/api/v10/users/@me"
        );
        assert!(client.url_for("https://evil.test/users/@me").is_err());
        assert!(client.url_for("//evil.test/users/@me").is_err());
    }
}
