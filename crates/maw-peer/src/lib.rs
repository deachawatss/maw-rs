//! Pure peer source resolution ported from maw-js `peer-sources.ts`.
//!
//! This crate does not perform network discovery. Callers pass already-fetched
//! scout discovery data, keeping the fixture-tested policy deterministic.

use std::{
    collections::BTreeMap,
    fmt::Write,
    fs, io,
    path::{Path, PathBuf},
};

use maw_xdg::{maw_state_path, MawXdgEnv};
use serde::{Deserialize, Serialize};

/// Peer source mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerSourceMode {
    Config,
    Scout,
    Both,
}

impl PeerSourceMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Scout => "scout",
            Self::Both => "both",
        }
    }
}

/// Peer target source kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerSourceKind {
    Config,
    Scout,
}

impl PeerSourceKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Scout => "scout",
        }
    }
}

/// Named peer from config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedPeerConfig {
    pub name: String,
    pub url: String,
}

/// Minimal maw config shape needed for peer source resolution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PeerConfig {
    pub peers: Vec<String>,
    pub named_peers: Vec<NamedPeerConfig>,
}

/// Resolved peer target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerTarget {
    pub name: Option<String>,
    pub url: String,
    pub source: PeerSourceKind,
    pub node: Option<String>,
    pub oracle: Option<String>,
}

/// Scout discovery row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiscoveryRow {
    pub node: Option<String>,
    pub oracle: Option<String>,
    pub host: Option<String>,
    pub locators: Vec<String>,
}

/// Discovery response supplied by runtime IO.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryResult {
    Ok { peers: Vec<DiscoveryRow> },
    Err { error: String, hint: Option<String> },
}

/// Peer source resolver result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerSourceResult {
    pub mode: PeerSourceMode,
    pub peers: Vec<PeerTarget>,
    pub warnings: Vec<String>,
    /// Number of discovery fetches the JS implementation would perform.
    pub fetch_calls: usize,
}

/// Parse a peer source mode value.
#[must_use]
pub fn parse_peer_source_mode(
    value: Option<&str>,
    fallback: PeerSourceMode,
) -> Option<PeerSourceMode> {
    match value {
        None | Some("") => Some(fallback),
        Some("config") => Some(PeerSourceMode::Config),
        Some("scout") => Some(PeerSourceMode::Scout),
        Some("both") => Some(PeerSourceMode::Both),
        Some(_) => None,
    }
}

/// Return configured peer targets with flat peers before named peers, deduped by URL.
#[must_use]
pub fn configured_peer_targets(config: &PeerConfig) -> Vec<PeerTarget> {
    let flat = config.peers.iter().map(|url| PeerTarget {
        name: None,
        url: url.clone(),
        source: PeerSourceKind::Config,
        node: None,
        oracle: None,
    });
    let named = config.named_peers.iter().map(|peer| PeerTarget {
        name: Some(peer.name.clone()),
        url: peer.url.clone(),
        source: PeerSourceKind::Config,
        node: None,
        oracle: None,
    });
    dedupe_peer_targets(flat.chain(named).collect())
}

/// Resolve config/scout peer sources from deterministic inputs.
#[must_use]
pub fn resolve_peer_sources(
    config: &PeerConfig,
    mode: PeerSourceMode,
    discoveries: Option<&DiscoveryResult>,
) -> PeerSourceResult {
    let config_peers = if mode == PeerSourceMode::Scout {
        Vec::new()
    } else {
        configured_peer_targets(config)
    };
    let mut warnings = Vec::new();
    let mut scout_peers = Vec::new();
    let mut fetch_calls = 0;

    if matches!(mode, PeerSourceMode::Scout | PeerSourceMode::Both) {
        fetch_calls = 1;
        match discoveries {
            Some(DiscoveryResult::Ok { peers }) => {
                scout_peers = peers.iter().filter_map(discovered_peer_target).collect();
            }
            Some(DiscoveryResult::Err { error, hint }) => {
                warnings.push(format_scout_warning(error, hint.as_deref()));
            }
            None => warnings.push("scout unavailable (missing_discoveries)".to_owned()),
        }
    }

    let peers = if mode == PeerSourceMode::Scout {
        scout_peers
    } else {
        let mut combined = config_peers;
        combined.extend(scout_peers);
        combined
    };

    PeerSourceResult {
        mode,
        peers: dedupe_peer_targets(peers),
        warnings,
        fetch_calls,
    }
}

/// Dedupe peer targets by URL after trimming trailing slashes.
#[must_use]
pub fn dedupe_peer_targets(peers: Vec<PeerTarget>) -> Vec<PeerTarget> {
    let mut seen: Vec<String> = Vec::new();
    let mut merged = Vec::new();
    for peer in peers {
        let key = peer_key(&peer.url);
        if seen.iter().any(|existing| existing == &key) {
            continue;
        }
        seen.push(key);
        merged.push(peer);
    }
    merged
}

fn discovered_peer_target(peer: &DiscoveryRow) -> Option<PeerTarget> {
    let url = peer.locators.iter().find(|locator| is_http_url(locator))?;
    Some(PeerTarget {
        name: peer.node.clone().or_else(|| peer.host.clone()),
        url: url.clone(),
        source: PeerSourceKind::Scout,
        node: peer.node.clone(),
        oracle: peer.oracle.clone(),
    })
}

fn is_http_url(value: &str) -> bool {
    let lower = value.to_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

fn peer_key(url: &str) -> String {
    url.trim_end_matches('/').to_owned()
}

fn format_scout_warning(error: &str, hint: Option<&str>) -> String {
    if let Some(hint) = hint {
        format!("scout unavailable ({error}: {hint})")
    } else {
        format!("scout unavailable ({error})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_applies_fallback_and_rejects_unknown() {
        assert_eq!(
            parse_peer_source_mode(None, PeerSourceMode::Both),
            Some(PeerSourceMode::Both)
        );
        assert_eq!(
            parse_peer_source_mode(Some(""), PeerSourceMode::Config),
            Some(PeerSourceMode::Config)
        );
        assert_eq!(
            parse_peer_source_mode(Some("scout"), PeerSourceMode::Both),
            Some(PeerSourceMode::Scout)
        );
        assert_eq!(
            parse_peer_source_mode(Some("invalid"), PeerSourceMode::Both),
            None
        );
    }
}

/// Structured peer probe failure code, ported from maw-js `probe.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProbeErrorCode {
    #[serde(rename = "DNS")]
    Dns,
    #[serde(rename = "REFUSED")]
    Refused,
    #[serde(rename = "TIMEOUT")]
    Timeout,
    #[serde(rename = "HTTP_4XX")]
    Http4xx,
    #[serde(rename = "HTTP_5XX")]
    Http5xx,
    #[serde(rename = "TLS")]
    Tls,
    #[serde(rename = "BAD_BODY")]
    BadBody,
    #[serde(rename = "UNKNOWN")]
    Unknown,
}

impl ProbeErrorCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dns => "DNS",
            Self::Refused => "REFUSED",
            Self::Timeout => "TIMEOUT",
            Self::Http4xx => "HTTP_4XX",
            Self::Http5xx => "HTTP_5XX",
            Self::Tls => "TLS",
            Self::BadBody => "BAD_BODY",
            Self::Unknown => "UNKNOWN",
        }
    }
}

/// Deterministic stand-in for JS `Response`/thrown-error shapes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeFailureInput {
    Http { status: u16, ok: bool },
    CauseCode(String),
    Code(String),
    Name(String),
    NonObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeLastError {
    pub code: ProbeErrorCode,
    pub message: String,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeMawHandshake {
    LegacyTrue,
    SchemaObject(String),
    EmptyObject,
    OtherTruthy,
    Missing,
}

#[must_use]
pub fn classify_probe_error(input: &ProbeFailureInput) -> ProbeErrorCode {
    match input {
        ProbeFailureInput::Http { status, ok } if !ok && (400..500).contains(status) => {
            ProbeErrorCode::Http4xx
        }
        ProbeFailureInput::Http { status, ok } if !ok && *status >= 500 => ProbeErrorCode::Http5xx,
        ProbeFailureInput::CauseCode(code) | ProbeFailureInput::Code(code) => classify_code(code),
        ProbeFailureInput::Name(name) if name == "AbortError" || name == "TimeoutError" => {
            ProbeErrorCode::Timeout
        }
        ProbeFailureInput::Http { .. }
        | ProbeFailureInput::NonObject
        | ProbeFailureInput::Name(_) => ProbeErrorCode::Unknown,
    }
}

fn classify_code(code: &str) -> ProbeErrorCode {
    match code {
        "ENOTFOUND" | "ENOTIMP" | "EAI_FAIL" | "EAI_AGAIN" | "EAI_NODATA" => ProbeErrorCode::Dns,
        "ECONNREFUSED" | "ConnectionRefused" => ProbeErrorCode::Refused,
        "ETIMEDOUT" | "UND_ERR_CONNECT_TIMEOUT" => ProbeErrorCode::Timeout,
        "UNABLE_TO_VERIFY_LEAF_SIGNATURE" => ProbeErrorCode::Tls,
        _ if code.starts_with("CERT_")
            || code.starts_with("SELF_SIGNED")
            || code.starts_with("DEPTH_ZERO_") =>
        {
            ProbeErrorCode::Tls
        }
        _ => ProbeErrorCode::Unknown,
    }
}

#[must_use]
pub const fn probe_exit_code(code: ProbeErrorCode) -> i32 {
    match code {
        ProbeErrorCode::Dns => 3,
        ProbeErrorCode::Refused => 4,
        ProbeErrorCode::Timeout => 5,
        ProbeErrorCode::Http4xx | ProbeErrorCode::Http5xx => 6,
        ProbeErrorCode::Tls | ProbeErrorCode::BadBody | ProbeErrorCode::Unknown => 2,
    }
}

#[must_use]
pub const fn probe_hint(code: ProbeErrorCode) -> &'static str {
    match code {
        ProbeErrorCode::Dns => "Host does not resolve. Check /etc/hosts, DNS, or VPN.",
        ProbeErrorCode::Refused => "Host resolves but port is closed. Is the peer process running?",
        ProbeErrorCode::Timeout => "Peer did not respond within 2s. Network path may be blocked.",
        ProbeErrorCode::Tls => "TLS handshake failed. Check cert validity / chain.",
        ProbeErrorCode::Http4xx => "Peer responded with a client error. /info endpoint may be missing OR peer is running an old maw version — if you control the peer, try restarting it.",
        ProbeErrorCode::Http5xx => "Peer returned a server error. Server-side fault.",
        ProbeErrorCode::BadBody => "/info responded but body shape was unexpected.",
        ProbeErrorCode::Unknown => "Probe failed for an unclassified reason.",
    }
}

#[must_use]
pub fn is_valid_maw_handshake(maw: &ProbeMawHandshake) -> bool {
    match maw {
        ProbeMawHandshake::LegacyTrue => true,
        ProbeMawHandshake::SchemaObject(schema) => !schema.is_empty(),
        ProbeMawHandshake::EmptyObject
        | ProbeMawHandshake::OtherTruthy
        | ProbeMawHandshake::Missing => false,
    }
}

#[must_use]
pub fn pick_probe_hint(err: &ProbeLastError) -> &'static str {
    if err.code == ProbeErrorCode::Dns && err.message.to_uppercase().contains("ENOTIMP") {
        return "install avahi-daemon (Linux) for mDNS, or add white.local to /etc/hosts";
    }
    probe_hint(err.code)
}

#[must_use]
pub fn format_probe_error(err: &ProbeLastError, url: &str, alias: &str) -> String {
    let hint = pick_probe_hint(err);
    let host = safe_probe_host(url);
    [
        format!(
            "\u{1b}[33m⚠\u{1b}[0m peer handshake failed: \u{1b}[1m{}\u{1b}[0m",
            err.code.as_str()
        ),
        format!("   host: {host}"),
        format!("   error: {}", err.message),
        format!("   hint: {hint}"),
        format!("   retry: maw peers probe {alias}"),
    ]
    .join("\n")
}

#[must_use]
pub fn safe_probe_host(url: &str) -> String {
    let Some(rest) = url.split_once("://").map(|(_, rest)| rest) else {
        return url.to_owned();
    };
    let host = rest.split('/').next().unwrap_or(rest);
    if host.is_empty() {
        url.to_owned()
    } else {
        host.to_owned()
    }
}

/// Parsed `/info` body shape for deterministic `probePeer` ports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeInfoBody {
    pub maw: ProbeMawHandshake,
    pub node: Option<String>,
    pub name: Option<String>,
    pub nickname: Option<String>,
}

/// Deterministic stand-in for the maw-js `/info` fetch result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeInfoOutcome {
    Body(ProbeInfoBody),
    HttpStatus { status: u16, ok: bool },
    InvalidJson,
    FetchCode { code: String, message: String },
    FetchCodeWithoutMessage { code: String },
    FetchName { name: String, message: String },
}

/// Deterministic stand-in for the best-effort `/api/identity` fetch result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeRemoteIdentity {
    Body {
        pubkey: Option<String>,
        oracle: Option<String>,
        node: Option<String>,
    },
    Missing,
    HttpError,
    MalformedJson,
    FetchError,
}

/// Peer's self-reported `<oracle>:<node>` identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerIdentity {
    pub oracle: String,
    pub node: String,
}

/// Deterministic plan input for maw-js `probePeer` runtime branches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbePeerPlan {
    pub url: String,
    pub now: String,
    pub dns_error: Option<ProbeLastError>,
    pub info: ProbeInfoOutcome,
    pub identity: Option<ProbeRemoteIdentity>,
}

/// Deterministic output for maw-js `probePeer`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbePeerResult {
    pub node: Option<String>,
    pub nickname: Option<String>,
    pub pubkey: Option<String>,
    pub identity: Option<PeerIdentity>,
    pub error: Option<ProbeLastError>,
}

/// Port of maw-js `probePeer` control flow over deterministic outcomes.
///
/// This deliberately stops short of real DNS/fetch IO; it locks the portable
/// branch behavior before the runtime adapter is wired.
#[must_use]
pub fn probe_peer_from_plan(plan: &ProbePeerPlan) -> ProbePeerResult {
    if let Some(err) = &plan.dns_error {
        return probe_failure(err.clone());
    }

    let body = match &plan.info {
        ProbeInfoOutcome::Body(body) => body,
        ProbeInfoOutcome::HttpStatus { status, ok } => {
            return probe_failure(ProbeLastError {
                code: classify_probe_error(&ProbeFailureInput::Http {
                    status: *status,
                    ok: *ok,
                }),
                message: format!("HTTP {status} from {}/info", plan.url),
                at: plan.now.clone(),
            });
        }
        ProbeInfoOutcome::InvalidJson => {
            return probe_bad_body("/info body was not valid JSON", &plan.now);
        }
        ProbeInfoOutcome::FetchCode { code, message } => {
            return probe_failure(ProbeLastError {
                code: classify_probe_error(&ProbeFailureInput::Code(code.clone())),
                message: message.clone(),
                at: plan.now.clone(),
            });
        }
        ProbeInfoOutcome::FetchCodeWithoutMessage { code } => {
            return probe_failure(ProbeLastError {
                code: classify_probe_error(&ProbeFailureInput::Code(code.clone())),
                message: format!("fetch {}/info failed", plan.url),
                at: plan.now.clone(),
            });
        }
        ProbeInfoOutcome::FetchName { name, message } => {
            return probe_failure(ProbeLastError {
                code: classify_probe_error(&ProbeFailureInput::Name(name.clone())),
                message: message.clone(),
                at: plan.now.clone(),
            });
        }
    };

    if !is_valid_maw_handshake(&body.maw) {
        return probe_bad_body(
            "/info response missing valid \"maw\" handshake field",
            &plan.now,
        );
    }

    let node = body
        .node
        .as_deref()
        .filter(|value| !value.is_empty())
        .or_else(|| body.name.as_deref().filter(|value| !value.is_empty()));
    let Some(node) = node else {
        return probe_bad_body(
            "/info response had neither \"node\" nor \"name\" string",
            &plan.now,
        );
    };

    let nickname = body
        .nickname
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let identity_fields = plan.identity.as_ref().and_then(parse_remote_identity);

    ProbePeerResult {
        node: Some(node.to_owned()),
        nickname,
        pubkey: identity_fields
            .as_ref()
            .and_then(|fields| fields.pubkey.clone()),
        identity: identity_fields.and_then(|fields| fields.identity),
        error: None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedRemoteIdentity {
    pubkey: Option<String>,
    identity: Option<PeerIdentity>,
}

fn parse_remote_identity(identity: &ProbeRemoteIdentity) -> Option<ParsedRemoteIdentity> {
    let ProbeRemoteIdentity::Body {
        pubkey,
        oracle,
        node,
    } = identity
    else {
        return None;
    };

    let pubkey = pubkey
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let node = node.as_deref().filter(|value| !value.is_empty());
    let identity = node.map(|node| PeerIdentity {
        oracle: oracle
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or("mawjs")
            .to_owned(),
        node: node.to_owned(),
    });

    Some(ParsedRemoteIdentity { pubkey, identity })
}

fn probe_bad_body(message: &str, now: &str) -> ProbePeerResult {
    probe_failure(ProbeLastError {
        code: ProbeErrorCode::BadBody,
        message: message.to_owned(),
        at: now.to_owned(),
    })
}

fn probe_failure(error: ProbeLastError) -> ProbePeerResult {
    ProbePeerResult {
        node: None,
        nickname: None,
        pubkey: None,
        identity: None,
        error: Some(error),
    }
}

/// Peer store record subset used by maw-js `probe-all`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerRecord {
    pub url: String,
    #[serde(default)]
    pub node: Option<String>,
    #[serde(rename = "addedAt")]
    pub added_at: String,
    #[serde(default, rename = "lastSeen")]
    pub last_seen: Option<String>,
    #[serde(default, rename = "lastError")]
    pub last_error: Option<ProbeLastError>,
}

/// Peer store file shape, ported from maw-js peers `store.ts` schema v1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerStoreFile {
    pub version: u8,
    #[serde(default)]
    pub peers: BTreeMap<String, PeerRecord>,
}

impl Default for PeerStoreFile {
    fn default() -> Self {
        Self {
            version: 1,
            peers: BTreeMap::new(),
        }
    }
}

/// Stale peer row used by doctor `--fix-stale` preview and mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StalePeer {
    pub alias: String,
    pub url: String,
    pub age_ms: Option<u64>,
}

/// Doctor check-shaped result for peers stale/fix-stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerDoctorCheck {
    pub name: String,
    pub ok: bool,
    pub message: String,
}

/// Deterministic peer-store environment for maw-js path resolution parity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerStoreEnv {
    xdg: MawXdgEnv,
}

impl PeerStoreEnv {
    #[must_use]
    pub fn new(home_dir: impl Into<PathBuf>) -> Self {
        Self {
            xdg: MawXdgEnv::new(home_dir),
        }
    }

    #[must_use]
    pub fn with_vars(
        home_dir: impl Into<PathBuf>,
        vars: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        Self {
            xdg: MawXdgEnv::with_vars(home_dir, vars),
        }
    }

    fn var(&self, name: &str) -> Option<&str> {
        self.xdg.var(name)
    }

    fn home_dir(&self) -> &Path {
        self.xdg.home_dir()
    }
}

#[must_use]
pub fn empty_peer_store() -> PeerStoreFile {
    PeerStoreFile::default()
}

/// Resolve the active `peers.json` path.
#[must_use]
pub fn peer_store_path(env: &PeerStoreEnv) -> PathBuf {
    env.var("PEERS_FILE")
        .map_or_else(|| maw_state_path(&env.xdg, &["peers.json"]), PathBuf::from)
}

fn legacy_peer_store_path(env: &PeerStoreEnv) -> Option<PathBuf> {
    if env.var("PEERS_FILE").is_some() || env.var("MAW_HOME").is_some() {
        return None;
    }
    let legacy = env.home_dir().join(".maw").join("peers.json");
    (legacy != peer_store_path(env)).then_some(legacy)
}

fn readable_peer_store_path(env: &PeerStoreEnv) -> PathBuf {
    let primary = peer_store_path(env);
    if primary.exists() {
        return primary;
    }
    legacy_peer_store_path(env)
        .filter(|path| path.exists())
        .unwrap_or(primary)
}

/// Load peers with stale tmp cleanup and corruption quarantine.
#[must_use]
pub fn load_peer_store(env: &PeerStoreEnv) -> PeerStoreFile {
    clear_stale_peer_store_tmp(env);
    let path = readable_peer_store_path(env);
    if !path.exists() {
        return empty_peer_store();
    }
    let Ok(raw) = fs::read_to_string(&path) else {
        return empty_peer_store();
    };
    match parse_peer_store(&raw) {
        Ok(store) => store,
        Err(error) => {
            let aside = corrupt_peer_store_path(&path);
            let _ = fs::rename(&path, aside);
            eprintln!(
                "\u{1b}[33m⚠\u{1b}[0m peers store at {} failed to parse ({error}); moved aside",
                path.display()
            );
            empty_peer_store()
        }
    }
}

/// Save peers via temp-file then rename, mirroring maw-js writeAtomic.
///
/// # Errors
///
/// Returns directory creation, JSON serialization, write, or rename failures.
pub fn save_peer_store(env: &PeerStoreEnv, data: &PeerStoreFile) -> io::Result<()> {
    let path = peer_store_path(env);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    write_peer_store_atomic(&path, data)
}

/// Read-modify-write peers, re-reading current contents before mutation.
///
/// # Errors
///
/// Returns directory creation, JSON serialization, write, or rename failures.
pub fn mutate_peer_store(
    env: &PeerStoreEnv,
    mutate: impl FnOnce(&mut PeerStoreFile),
) -> io::Result<PeerStoreFile> {
    let path = peer_store_path(env);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let read_path = if path.exists() {
        path.clone()
    } else {
        readable_peer_store_path(env)
    };
    let mut store = read_peer_store_unlocked(&read_path);
    mutate(&mut store);
    write_peer_store_atomic(&path, &store)?;
    Ok(store)
}

/// Best-effort stale `.tmp` cleanup for primary and legacy peer stores.
pub fn clear_stale_peer_store_tmp(env: &PeerStoreEnv) {
    for path in [Some(peer_store_path(env)), legacy_peer_store_path(env)]
        .into_iter()
        .flatten()
    {
        let _ = fs::remove_file(tmp_peer_store_path(&path));
    }
}

/// Enumerate stale peers from the peer store in stable alias order.
#[must_use]
pub fn stale_peers(env: &PeerStoreEnv, now_ms: u64) -> Vec<StalePeer> {
    let ttl_ms = parse_stale_ttl_ms(env.var("MAW_PEER_STALE_TTL_MS"));
    load_peer_store(env)
        .peers
        .into_iter()
        .filter(|(_, peer)| is_peer_stale(peer, ttl_ms, now_ms))
        .map(|(alias, peer)| {
            let age_ms = stale_age_ms(&peer, now_ms);
            StalePeer {
                alias,
                url: peer.url,
                age_ms,
            }
        })
        .collect()
}

/// Return the maw-js `peers:stale` doctor check shape.
#[must_use]
pub fn stale_peer_check(env: &PeerStoreEnv, now_ms: u64) -> PeerDoctorCheck {
    let stale = stale_peers(env, now_ms);
    if stale.is_empty() {
        return PeerDoctorCheck {
            name: "peers:stale".to_owned(),
            ok: true,
            message: "no stale peers".to_owned(),
        };
    }
    let days = parse_stale_ttl_ms(env.var("MAW_PEER_STALE_TTL_MS")) / 86_400_000;
    PeerDoctorCheck {
        name: "peers:stale".to_owned(),
        ok: false,
        message: format!(
            "{} stale peer{} (>{days}d) — run 'maw doctor --fix-stale' to remove",
            stale.len(),
            if stale.len() == 1 { "" } else { "s" }
        ),
    }
}

/// Remove stale peers through the peer-store mutation path.
///
/// # Errors
///
/// Returns peer-store mutation write failures.
pub fn remove_stale_peers(env: &PeerStoreEnv, now_ms: u64) -> io::Result<PeerDoctorCheck> {
    let stale = stale_peers(env, now_ms);
    if stale.is_empty() {
        return Ok(PeerDoctorCheck {
            name: "peers:fix-stale".to_owned(),
            ok: true,
            message: "no stale peers".to_owned(),
        });
    }
    let mut removed = 0;
    mutate_peer_store(env, |data| {
        for stale_peer in &stale {
            if data.peers.remove(&stale_peer.alias).is_some() {
                removed += 1;
            }
        }
    })?;
    Ok(PeerDoctorCheck {
        name: "peers:fix-stale".to_owned(),
        ok: true,
        message: format!(
            "removed {removed} stale peer{}",
            if removed == 1 { "" } else { "s" }
        ),
    })
}

fn read_peer_store_unlocked(path: &Path) -> PeerStoreFile {
    if !path.exists() {
        return empty_peer_store();
    }
    let Ok(raw) = fs::read_to_string(path) else {
        return empty_peer_store();
    };
    parse_peer_store(&raw).unwrap_or_else(|_| empty_peer_store())
}

fn parse_peer_store(raw: &str) -> Result<PeerStoreFile, String> {
    let value = serde_json::from_str::<serde_json::Value>(raw).map_err(|err| err.to_string())?;
    let peers = match value.get("peers") {
        Some(peers) if peers.is_object() => peers.clone(),
        Some(_) => {
            return Err("invalid store shape (expected { peers: { ... } } object)".to_owned());
        }
        None => serde_json::json!({}),
    };
    if !peers.is_object() {
        return Err("invalid store shape (expected { peers: { ... } } object)".to_owned());
    }
    serde_json::from_value(serde_json::json!({ "version": 1, "peers": peers }))
        .map_err(|err| err.to_string())
}

fn write_peer_store_atomic(path: &Path, data: &PeerStoreFile) -> io::Result<()> {
    let tmp = tmp_peer_store_path(path);
    let json = serde_json::to_string_pretty(data).map_err(io::Error::other)?;
    fs::write(&tmp, format!("{json}\n"))?;
    fs::rename(tmp, path)
}

fn tmp_peer_store_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.tmp", path.display()))
}

fn corrupt_peer_store_path(path: &Path) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    PathBuf::from(format!("{}.corrupt-{stamp}", path.display()))
}

/// Deterministic input for maw-js `cmdProbeAll`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeAllPlan {
    pub timeout_ms: u64,
    pub now: String,
    pub peers: Vec<(String, PeerRecord)>,
    /// URL → probe result → elapsed milliseconds.
    pub probe_results: Vec<(String, ProbePeerResult, u64)>,
    /// Aliases removed after load and before mutation.
    pub removed_before_mutate: Vec<String>,
}

/// Renderable per-peer probe-all row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeAllRow {
    pub alias: String,
    pub url: String,
    pub node: Option<String>,
    pub last_seen: Option<String>,
    pub ok: bool,
    pub ms: u64,
    pub error: Option<ProbeLastError>,
}

/// Deterministic result for maw-js `cmdProbeAll`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeAllResult {
    pub rows: Vec<ProbeAllRow>,
    pub ok_count: usize,
    pub fail_count: usize,
    pub worst_exit_code: i32,
    pub probe_calls: Vec<(String, u64)>,
    pub mutate_calls: usize,
    pub peers_after: BTreeMap<String, PeerRecord>,
}

/// Port of maw-js `cmdProbeAll` over deterministic store/probe inputs.
#[must_use]
pub fn probe_all_from_plan(plan: &ProbeAllPlan) -> ProbeAllResult {
    let mut peers_after: BTreeMap<String, PeerRecord> = plan.peers.iter().cloned().collect();
    let probe_results: BTreeMap<String, (ProbePeerResult, u64)> = plan
        .probe_results
        .iter()
        .map(|(url, result, ms)| (url.clone(), (result.clone(), *ms)))
        .collect();
    let mut entries = plan.peers.clone();
    entries.sort_by(|(left, _), (right, _)| left.cmp(right));

    let mut probe_calls = Vec::with_capacity(entries.len());
    let mut rows = Vec::with_capacity(entries.len());
    for (alias, peer) in entries {
        probe_calls.push((peer.url.clone(), plan.timeout_ms));
        let (probe, ms) = probe_results
            .get(&peer.url)
            .cloned()
            .unwrap_or_else(|| (probe_failure_without_error(), 0));
        let error = probe.error.clone();
        rows.push(ProbeAllRow {
            alias,
            url: peer.url,
            node: probe.node.or(peer.node),
            last_seen: peer.last_seen,
            ok: error.is_none(),
            ms,
            error,
        });
    }

    let mutate_calls = usize::from(!rows.is_empty());
    if mutate_calls == 1 {
        for alias in &plan.removed_before_mutate {
            peers_after.remove(alias);
        }
        for row in &mut rows {
            let Some(peer) = peers_after.get_mut(&row.alias) else {
                continue;
            };
            if row.ok {
                peer.last_error = None;
                peer.last_seen = Some(plan.now.clone());
                row.last_seen = Some(plan.now.clone());
                if let Some(node) = &row.node {
                    peer.node = Some(node.clone());
                }
            } else if let Some(error) = &row.error {
                peer.last_error = Some(error.clone());
            }
        }
    }

    let ok_count = rows.iter().filter(|row| row.ok).count();
    let fail_count = rows.len() - ok_count;
    let worst_exit_code = rows
        .iter()
        .filter_map(|row| row.error.as_ref())
        .map(|err| probe_exit_code(err.code))
        .max()
        .unwrap_or(0);

    ProbeAllResult {
        rows,
        ok_count,
        fail_count,
        worst_exit_code,
        probe_calls,
        mutate_calls,
        peers_after,
    }
}

fn probe_failure_without_error() -> ProbePeerResult {
    ProbePeerResult {
        node: None,
        nickname: None,
        pubkey: None,
        identity: None,
        error: None,
    }
}

/// Render maw-js `formatProbeAll` table output.
#[must_use]
pub fn format_probe_all(result: &ProbeAllResult) -> String {
    if result.rows.is_empty() {
        return "no peers".to_owned();
    }

    let header = ["alias", "url", "node", "lastSeen", "result"].map(str::to_owned);
    let rows: Vec<[String; 5]> = result
        .rows
        .iter()
        .map(|row| {
            [
                row.alias.clone(),
                row.url.clone(),
                row.node.clone().unwrap_or_else(|| "-".to_owned()),
                row.last_seen.clone().unwrap_or_else(|| "-".to_owned()),
                if row.ok {
                    format!("\u{1b}[32m✓\u{1b}[0m ok ({}ms)", row.ms)
                } else {
                    format!(
                        "\u{1b}[31m✗\u{1b}[0m {}",
                        row.error
                            .as_ref()
                            .map_or("UNKNOWN", |err| err.code.as_str())
                    )
                },
            ]
        })
        .collect();

    let widths: Vec<usize> = header
        .iter()
        .enumerate()
        .map(|(index, heading)| {
            rows.iter()
                .map(|row| ansi_stripped_len(&row[index]))
                .max()
                .unwrap_or(0)
                .max(heading.len())
        })
        .collect();

    let divider = widths
        .iter()
        .map(|width| "-".repeat(*width))
        .collect::<Vec<_>>();
    let mut lines = vec![
        format_probe_all_row(&header, &widths),
        format_probe_all_row(&divider, &widths),
    ];
    lines.extend(rows.iter().map(|row| format_probe_all_row(row, &widths)));
    lines.push(String::new());
    lines.push(format!(
        "{}/{} ok{}",
        result.ok_count,
        result.rows.len(),
        if result.fail_count > 0 {
            format!(", {} failed", result.fail_count)
        } else {
            String::new()
        }
    ));
    lines.join("\n")
}

fn format_probe_all_row(cols: &[String], widths: &[usize]) -> String {
    cols.iter()
        .enumerate()
        .map(|(index, col)| {
            let padding = widths[index].saturating_sub(ansi_stripped_len(col));
            format!("{col}{}", " ".repeat(padding))
        })
        .collect::<Vec<_>>()
        .join("  ")
}

fn ansi_stripped_len(value: &str) -> usize {
    let mut len = 0;
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for code_ch in chars.by_ref() {
                if code_ch == 'm' {
                    break;
                }
            }
        } else {
            len += ch.len_utf8();
        }
    }
    len
}

/// Validate a peer alias using maw-js `impl.ts` rules.
#[must_use]
pub fn validate_peer_alias(alias: &str) -> Option<String> {
    if is_valid_peer_alias(alias) {
        None
    } else {
        Some(format!(
            "invalid alias \"{alias}\" (must match ^[a-z0-9][a-z0-9_-]{{0,31}}$)"
        ))
    }
}

fn is_valid_peer_alias(alias: &str) -> bool {
    let mut chars = alias.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    let rest_len = chars
        .try_fold(0usize, |count, ch| {
            if ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '_' | '-') {
                Some(count + 1)
            } else {
                None
            }
        })
        .unwrap_or(usize::MAX);
    rest_len <= 31
}

/// Validate a peer URL using maw-js `impl.ts` rules.
#[must_use]
pub fn validate_peer_url(raw: &str) -> Option<String> {
    let Some((protocol, rest)) = raw.split_once("://") else {
        return Some(format!("invalid URL \"{raw}\""));
    };
    if !matches!(protocol, "http" | "https") {
        return Some(format!(
            "invalid URL \"{raw}\" (must be http:// or https://)"
        ));
    }
    let host = rest.split('/').next().unwrap_or_default();
    if host.is_empty() || host.chars().any(char::is_whitespace) {
        return Some(format!("invalid URL \"{raw}\""));
    }
    None
}

/// Renderable peer-list row, ported from maw-js `PeerListRow`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerListRow {
    pub alias: String,
    pub url: String,
    pub node: Option<String>,
    pub nickname: Option<String>,
    pub last_seen: Option<String>,
    pub stale: bool,
    pub stale_age_ms: Option<u64>,
}

/// Render maw-js `formatList` output for peer rows.
#[must_use]
pub fn format_peer_list(rows: &[PeerListRow]) -> String {
    if rows.is_empty() {
        return "no peers".to_owned();
    }

    let header = ["alias", "url", "node", "nickname", "lastSeen"].map(str::to_owned);
    let lines: Vec<[String; 5]> = rows
        .iter()
        .map(|row| {
            [
                row.alias.clone(),
                row.url.clone(),
                row.node.clone().unwrap_or_else(|| "-".to_owned()),
                row.nickname.clone().unwrap_or_else(|| "-".to_owned()),
                row.last_seen.clone().unwrap_or_else(|| "-".to_owned()),
            ]
        })
        .collect();
    let widths: Vec<usize> = header
        .iter()
        .enumerate()
        .map(|(index, heading)| {
            lines
                .iter()
                .map(|line| line[index].len())
                .max()
                .unwrap_or(0)
                .max(heading.len())
        })
        .collect();

    let divider = widths
        .iter()
        .map(|width| "-".repeat(*width))
        .collect::<Vec<_>>();
    let mut out = vec![
        format_peer_list_row(&header, &widths),
        format_peer_list_row(&divider, &widths),
    ];
    out.extend(rows.iter().zip(lines.iter()).map(|(row, line)| {
        let mut rendered = format_peer_list_row(line, &widths);
        if row.stale {
            let suffix = row.stale_age_ms.map_or_else(
                || "never seen".to_owned(),
                |age| format!("last seen {}d ago", age / (24 * 60 * 60 * 1000)),
            );
            let _ = write!(rendered, "  \u{1b}[2m(stale, {suffix})\u{1b}[0m");
        }
        rendered
    }));
    out.join("\n")
}

fn format_peer_list_row(cols: &[String], widths: &[usize]) -> String {
    cols.iter()
        .enumerate()
        .map(|(index, col)| {
            format!(
                "{col}{}",
                " ".repeat(widths[index].saturating_sub(col.len()))
            )
        })
        .collect::<Vec<_>>()
        .join("  ")
}

/// Default maw-js stale peer TTL: 7 days in milliseconds.
#[must_use]
pub const fn default_stale_ttl_ms() -> u64 {
    7 * 24 * 60 * 60 * 1000
}

/// Resolve stale TTL from `MAW_PEER_STALE_TTL_MS`-style input.
#[must_use]
pub fn parse_stale_ttl_ms(raw: Option<&str>) -> u64 {
    let Some(raw) = raw.filter(|value| !value.is_empty()) else {
        return default_stale_ttl_ms();
    };
    raw.parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .unwrap_or_else(default_stale_ttl_ms)
}

/// Age of a peer's most informative timestamp in milliseconds.
///
/// Mirrors maw-js: use `lastSeen` when present, otherwise `addedAt`; invalid
/// provenance returns `None`, and future timestamps clamp to `0`.
#[must_use]
pub fn stale_age_ms(peer: &PeerRecord, now_ms: u64) -> Option<u64> {
    let reference = peer.last_seen.as_deref().unwrap_or(&peer.added_at);
    let timestamp = parse_iso_timestamp_ms(reference)?;
    Some(now_ms.saturating_sub(timestamp))
}

/// Is a peer stale for a given TTL and wall-clock timestamp?
#[must_use]
pub fn is_peer_stale(peer: &PeerRecord, ttl_ms: u64, now_ms: u64) -> bool {
    stale_age_ms(peer, now_ms).is_none_or(|age| age > ttl_ms)
}

fn parse_iso_timestamp_ms(value: &str) -> Option<u64> {
    let (date, time) = value.strip_suffix('Z')?.split_once('T')?;
    let mut date_parts = date.split('-');
    let year = date_parts.next()?.parse::<i32>().ok()?;
    let month = date_parts.next()?.parse::<u32>().ok()?;
    let day = date_parts.next()?.parse::<u32>().ok()?;
    if date_parts.next().is_some() {
        return None;
    }

    let mut time_parts = time.split(':');
    let hour = time_parts.next()?.parse::<u32>().ok()?;
    let minute = time_parts.next()?.parse::<u32>().ok()?;
    let second_part = time_parts.next()?;
    if time_parts.next().is_some() {
        return None;
    }
    let (second_raw, millis_raw) = second_part.split_once('.').unwrap_or((second_part, "0"));
    let second = second_raw.parse::<u32>().ok()?;
    let millis = parse_millis(millis_raw)?;

    if !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }

    let days = days_from_civil(year, month, day)?;
    let seconds = days
        .checked_mul(86_400)?
        .checked_add(i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second))?;
    let ms = seconds.checked_mul(1000)?.checked_add(i64::from(millis))?;
    u64::try_from(ms).ok()
}

fn parse_millis(raw: &str) -> Option<u32> {
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let mut value = raw.chars().take(3).collect::<String>();
    while value.len() < 3 {
        value.push('0');
    }
    value.parse::<u32>().ok()
}

const fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

const fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Days since Unix epoch for a Gregorian date.
fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    let year = year - i32::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month_i = i32::try_from(month).ok()?;
    let doy =
        (153 * (month_i + if month_i > 2 { -3 } else { 9 }) + 2) / 5 + i32::try_from(day).ok()? - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(i64::from(era) * 146_097 + i64::from(doe) - 719_468)
}
