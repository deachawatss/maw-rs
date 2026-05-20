//! Pure peer source resolution ported from maw-js `peer-sources.ts`.
//!
//! This crate does not perform network discovery. Callers pass already-fetched
//! scout discovery data, keeping the fixture-tested policy deterministic.

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeErrorCode {
    Dns,
    Refused,
    Timeout,
    Http4xx,
    Http5xx,
    Tls,
    BadBody,
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
