//! Federation auth pure helpers ported from maw-js `src/lib/federation-auth.ts`.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

type HmacSha256 = Hmac<Sha256>;

pub const WINDOW_SEC: i64 = 300;
pub const DEFAULT_ORACLE: &str = "mawjs";
pub const PAIR_CODE_ALPHABET: &str = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Headers(BTreeMap<String, String>);

impl Headers {
    #[must_use]
    pub fn new(entries: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>) -> Self {
        let mut map = BTreeMap::new();
        for (key, value) in entries {
            map.insert(key.into().to_lowercase(), value.into());
        }
        Self(map)
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(&name.to_lowercase()).map(String::as_str)
    }

    #[must_use]
    pub fn to_btree_map(&self) -> BTreeMap<String, String> {
        self.0.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V3Signature {
    pub signature: String,
    pub body_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FromAddressConfig {
    pub oracle: Option<String>,
    pub node: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoPairIdentity {
    pub node: String,
    pub oracle: String,
    pub url: String,
    pub pubkey: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConsentAction {
    Hey,
    TeamInvite,
    PluginInstall,
}

impl ConsentAction {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hey => "hey",
            Self::TeamInvite => "team-invite",
            Self::PluginInstall => "plugin-install",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovedBy {
    Human,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentStatus {
    Pending,
    Approved,
    Rejected,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustEntry {
    pub from: String,
    pub to: String,
    pub action: ConsentAction,
    pub approved_at: String,
    pub approved_by: ApprovedBy,
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRequest {
    pub id: String,
    pub from: String,
    pub to: String,
    pub action: ConsentAction,
    pub summary: String,
    pub pin_hash: String,
    pub created_at: String,
    pub expires_at: String,
    pub status: ConsentStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerPendingRequest {
    pub id: String,
    pub from: String,
    pub to: String,
    pub action: ConsentAction,
    pub summary: String,
    pub pin_hash: String,
    pub created_at: String,
    pub expires_at: String,
    pub status: ConsentStatus,
    pub pin: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerPostResult {
    Skipped,
    Ok,
    HttpStatus(u16),
    NetworkError(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsentRequestArgs {
    pub from: String,
    pub to: String,
    pub action: ConsentAction,
    pub summary: String,
    pub peer_url: Option<String>,
    pub request_id: String,
    pub pin: String,
    pub now_ms: i64,
    pub peer_post: PeerPostResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsentRequestResult {
    pub ok: bool,
    pub request_id: Option<String>,
    pub pin: Option<String>,
    pub expires_at: Option<String>,
    pub error: Option<String>,
    pub already_trusted: bool,
    pub peer_url: Option<String>,
    pub peer_method: Option<String>,
    pub peer_body: Option<PeerPendingRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsentApprovalResult {
    pub ok: bool,
    pub error: Option<String>,
    pub entry: Option<TrustEntry>,
}

#[derive(Debug, Clone, Default)]
pub struct ConsentStore {
    trust: BTreeMap<String, TrustEntry>,
    pending: BTreeMap<String, PendingRequest>,
}

impl ConsentStore {
    pub fn record_trust(&mut self, entry: TrustEntry) {
        self.trust
            .insert(trust_key(&entry.from, &entry.to, entry.action), entry);
    }

    #[must_use]
    pub fn remove_trust(&mut self, from: &str, to: &str, action: ConsentAction) -> bool {
        self.trust.remove(&trust_key(from, to, action)).is_some()
    }

    #[must_use]
    pub fn is_trusted(&self, from: &str, to: &str, action: ConsentAction) -> bool {
        self.trust.contains_key(&trust_key(from, to, action))
    }

    #[must_use]
    pub fn list_trust(&self) -> Vec<TrustEntry> {
        let mut entries = self.trust.values().cloned().collect::<Vec<_>>();
        entries.sort_by(|a, b| a.approved_at.cmp(&b.approved_at));
        entries
    }

    pub fn write_pending(&mut self, request: PendingRequest) {
        self.pending.insert(request.id.clone(), request);
    }

    #[must_use]
    pub fn read_pending(&self, id: &str) -> Option<PendingRequest> {
        self.pending.get(id).cloned()
    }

    #[must_use]
    pub fn list_pending(&self) -> Vec<PendingRequest> {
        let mut entries = self.pending.values().cloned().collect::<Vec<_>>();
        entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        entries
    }

    pub fn update_status(&mut self, id: &str, status: ConsentStatus) -> bool {
        let Some(request) = self.pending.get_mut(id) else {
            return false;
        };
        request.status = status;
        true
    }

    pub fn delete_pending(&mut self, id: &str) -> bool {
        self.pending.remove(id).is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairEntry {
    pub code: String,
    pub expires_at: u64,
    pub consumed: bool,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupResult {
    Live(PairEntry),
    NotFound,
    Expired,
    Consumed,
}

#[derive(Debug, Clone, Default)]
pub struct PairCodeStore {
    entries: BTreeMap<String, PairEntry>,
}

impl PairCodeStore {
    #[must_use]
    pub fn register_at(&mut self, code: &str, ttl_ms: u64, now_ms: u64) -> PairEntry {
        let entry = PairEntry {
            code: normalize_pair_code(code),
            expires_at: now_ms.saturating_add(ttl_ms),
            consumed: false,
            created_at: now_ms,
        };
        self.entries.insert(entry.code.clone(), entry.clone());
        entry
    }

    #[must_use]
    pub fn lookup_at(&self, code: &str, now_ms: u64) -> LookupResult {
        let Some(entry) = self.entries.get(&normalize_pair_code(code)).cloned() else {
            return LookupResult::NotFound;
        };
        if entry.consumed {
            return LookupResult::Consumed;
        }
        if now_ms > entry.expires_at {
            return LookupResult::Expired;
        }
        LookupResult::Live(entry)
    }

    #[must_use]
    pub fn consume_at(&mut self, code: &str, now_ms: u64) -> LookupResult {
        let key = normalize_pair_code(code);
        match self.lookup_at(&key, now_ms) {
            LookupResult::Live(mut entry) => {
                entry.consumed = true;
                self.entries.insert(key, entry.clone());
                LookupResult::Live(entry)
            }
            other => other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FromVerifyDecision {
    AcceptLegacy {
        reason: String,
    },
    AcceptTofuRecord {
        reason: String,
        from: String,
    },
    AcceptVerified {
        reason: String,
        from: String,
    },
    RefuseUnsigned {
        reason: String,
        from: Option<String>,
    },
    RefuseMismatch {
        reason: String,
        from: String,
    },
    RefuseSkew {
        reason: String,
        from: String,
        delta: i64,
    },
    RefuseMalformed {
        reason: String,
    },
}

impl FromVerifyDecision {
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::AcceptLegacy { .. } => "accept-legacy",
            Self::AcceptTofuRecord { .. } => "accept-tofu-record",
            Self::AcceptVerified { .. } => "accept-verified",
            Self::RefuseUnsigned { .. } => "refuse-unsigned",
            Self::RefuseMismatch { .. } => "refuse-mismatch",
            Self::RefuseSkew { .. } => "refuse-skew",
            Self::RefuseMalformed { .. } => "refuse-malformed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyRequestArgs {
    pub method: String,
    pub path: String,
    pub headers: Headers,
    pub body: Option<Vec<u8>>,
    pub cached_pubkey: Option<String>,
    pub now: i64,
}

#[must_use]
pub fn hash_body(body: Option<&[u8]>) -> String {
    let Some(body) = body else {
        return String::new();
    };
    if body.is_empty() {
        return String::new();
    }
    hex_lower(&Sha256::digest(body))
}

#[must_use]
pub fn sign(token: &str, method: &str, path: &str, timestamp: i64, body_hash: &str) -> String {
    let payload = if body_hash.is_empty() {
        format!("{method}:{path}:{timestamp}")
    } else {
        format!("{method}:{path}:{timestamp}:{body_hash}")
    };
    hmac_sha256_hex(token, &payload)
}

#[must_use]
pub fn verify(
    token: &str,
    method: &str,
    path: &str,
    timestamp: i64,
    signature: &str,
    body_hash: &str,
    now: i64,
) -> bool {
    let delta = (now - timestamp).abs();
    if delta > WINDOW_SEC {
        return false;
    }
    let expected = sign(token, method, path, timestamp, body_hash);
    expected.len() == signature.len() && constant_time_eq(expected.as_bytes(), signature.as_bytes())
}

#[must_use]
pub fn is_loopback(address: Option<&str>) -> bool {
    let Some(address) = address else {
        return false;
    };
    address == "127.0.0.1"
        || address == "::1"
        || address == "::ffff:127.0.0.1"
        || address == "localhost"
        || address.starts_with("127.")
}

#[must_use]
pub fn sign_headers_at(
    token: &str,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
    timestamp: i64,
) -> Headers {
    let body_hash = body.map_or_else(String::new, |body| hash_body(Some(body)));
    let mut headers = vec![
        ("X-Maw-Timestamp".to_owned(), timestamp.to_string()),
        (
            "X-Maw-Signature".to_owned(),
            sign(token, method, path, timestamp, &body_hash),
        ),
    ];
    if !body_hash.is_empty() {
        headers.push(("X-Maw-Auth-Version".to_owned(), "v2".to_owned()));
    }
    Headers::new(headers)
}

/// Sign the v3 `from:` request payload.
///
/// # Errors
///
/// Returns an error when `peer_key` or `from_address` is empty, matching maw-js's
/// loud validation branches.
pub fn sign_request_v3(
    peer_key: &str,
    from_address: &str,
    method: &str,
    path: &str,
    timestamp: i64,
    body: Option<&[u8]>,
) -> Result<V3Signature, String> {
    if peer_key.is_empty() {
        return Err("signRequestV3: peerKey is required".to_owned());
    }
    if from_address.is_empty() {
        return Err("signRequestV3: fromAddress is required (<oracle>:<node>)".to_owned());
    }
    let method = if method.is_empty() { "GET" } else { method }.to_uppercase();
    let body_hash = body.map_or_else(String::new, |body| hash_body(Some(body)));
    let payload = build_from_sign_payload(from_address, timestamp, &method, path, &body_hash);
    Ok(V3Signature {
        signature: hmac_sha256_hex(peer_key, &payload),
        body_hash,
    })
}

/// Produce v3 outbound auth headers.
///
/// # Errors
///
/// Returns an error when v3 signing inputs are invalid.
pub fn sign_headers_v3_at(
    peer_key: &str,
    from_address: &str,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
    timestamp: i64,
) -> Result<Headers, String> {
    let signature = sign_request_v3(peer_key, from_address, method, path, timestamp, body)?;
    Ok(Headers::new([
        ("X-Maw-From", from_address.to_owned()),
        ("X-Maw-Signature-V3", signature.signature),
        ("X-Maw-Timestamp", timestamp.to_string()),
        ("X-Maw-Auth-Version", "v3".to_owned()),
    ]))
}

#[must_use]
pub fn resolve_from_address(config: &FromAddressConfig) -> Option<String> {
    let node = config.node.as_deref()?;
    let oracle = config.oracle.as_deref().unwrap_or(DEFAULT_ORACLE);
    Some(format!("{oracle}:{node}"))
}

#[must_use]
pub fn normalize_pair_code(raw: &str) -> String {
    raw.chars()
        .filter(|ch| *ch != '-' && !ch.is_whitespace())
        .flat_map(char::to_uppercase)
        .collect()
}

#[must_use]
pub fn is_valid_pair_code_shape(code: &str) -> bool {
    let code = normalize_pair_code(code);
    code.len() == 6 && code.chars().all(|ch| PAIR_CODE_ALPHABET.contains(ch))
}

#[must_use]
pub fn pretty_pair_code(code: &str) -> String {
    let code = normalize_pair_code(code);
    if code.len() == 6 {
        format!("{}-{}", &code[..3], &code[3..])
    } else {
        code
    }
}

#[must_use]
pub fn redact_pair_code(code: &str) -> String {
    let code = normalize_pair_code(code);
    if code.len() >= 3 {
        format!("{}-***", &code[..3])
    } else {
        "***".to_owned()
    }
}

#[must_use]
pub fn generate_pair_code_from_bytes(bytes: &[u8]) -> String {
    let alphabet = PAIR_CODE_ALPHABET.as_bytes();
    bytes
        .iter()
        .take(6)
        .map(|byte| char::from(alphabet[usize::from(byte % 32)]))
        .collect()
}

#[must_use]
pub fn hash_consent_pin(pin: &str) -> String {
    hex_lower(&Sha256::digest(normalize_pair_code(pin).as_bytes()))
}

#[must_use]
pub fn verify_consent_pin(pin: &str, expected_hash: &str) -> bool {
    is_valid_pair_code_shape(pin) && hash_consent_pin(pin) == expected_hash
}

#[must_use]
pub fn consent_request_id_from_bytes(bytes: &[u8]) -> String {
    hex_lower(&bytes.iter().copied().take(12).collect::<Vec<_>>())
}

#[must_use]
pub fn trust_key(from: &str, to: &str, action: ConsentAction) -> String {
    format!("{from}→{to}:{}", action.as_str())
}

#[must_use]
pub fn apply_consent_expiry(request: &PendingRequest, now_ms: i64) -> PendingRequest {
    if request.status == ConsentStatus::Pending
        && parse_iso_millis(&request.expires_at).is_some_and(|expires_at| now_ms > expires_at)
    {
        return PendingRequest {
            status: ConsentStatus::Expired,
            ..request.clone()
        };
    }
    request.clone()
}

pub fn request_consent_plan(
    store: &mut ConsentStore,
    args: ConsentRequestArgs,
) -> ConsentRequestResult {
    const TTL_MS: i64 = 10 * 60 * 1_000;
    let created_at = iso_from_unix_millis(args.now_ms);
    let expires_at = iso_from_unix_millis(args.now_ms.saturating_add(TTL_MS));
    let pending = PendingRequest {
        id: args.request_id.clone(),
        from: args.from,
        to: args.to,
        action: args.action,
        summary: args.summary,
        pin_hash: hash_consent_pin(&args.pin),
        created_at,
        expires_at: expires_at.clone(),
        status: ConsentStatus::Pending,
    };
    store.write_pending(pending.clone());

    let peer_url = args
        .peer_url
        .as_deref()
        .map(|url| format!("{}/api/consent/request", url.trim_end_matches('/')));
    let peer_body = peer_url.as_ref().map(|_| PeerPendingRequest {
        id: pending.id.clone(),
        from: pending.from.clone(),
        to: pending.to.clone(),
        action: pending.action,
        summary: pending.summary.clone(),
        pin_hash: pending.pin_hash.clone(),
        created_at: pending.created_at.clone(),
        expires_at: pending.expires_at.clone(),
        status: pending.status,
        pin: None,
    });

    match args.peer_post {
        PeerPostResult::HttpStatus(status) if status >= 400 => ConsentRequestResult {
            ok: false,
            request_id: Some(args.request_id),
            pin: None,
            expires_at: Some(expires_at),
            error: Some(format!("peer rejected request: HTTP {status}")),
            already_trusted: false,
            peer_url,
            peer_method: Some("POST".to_owned()),
            peer_body,
        },
        PeerPostResult::NetworkError(message) => ConsentRequestResult {
            ok: false,
            request_id: Some(args.request_id),
            pin: None,
            expires_at: Some(expires_at),
            error: Some(format!("network error contacting peer: {message}")),
            already_trusted: false,
            peer_url,
            peer_method: Some("POST".to_owned()),
            peer_body,
        },
        PeerPostResult::Skipped | PeerPostResult::Ok | PeerPostResult::HttpStatus(_) => {
            ConsentRequestResult {
                ok: true,
                request_id: Some(args.request_id),
                pin: Some(args.pin),
                expires_at: Some(expires_at),
                error: None,
                already_trusted: false,
                peer_method: peer_url.as_ref().map(|_| "POST".to_owned()),
                peer_url,
                peer_body,
            }
        }
    }
}

pub fn approve_consent_plan(
    store: &mut ConsentStore,
    request_id: &str,
    pin: &str,
    now_ms: i64,
) -> ConsentApprovalResult {
    let Some(request) = store.read_pending(request_id) else {
        return consent_error(format!("request not found: {request_id}"));
    };
    let request = apply_consent_expiry(&request, now_ms);
    if request.status != ConsentStatus::Pending {
        return consent_error(format!(
            "request is {}, cannot approve",
            consent_status_str(request.status)
        ));
    }
    if !verify_consent_pin(pin, &request.pin_hash) {
        return consent_error("PIN mismatch");
    }
    store.update_status(request_id, ConsentStatus::Approved);
    let entry = TrustEntry {
        from: request.from,
        to: request.to,
        action: request.action,
        approved_at: iso_from_unix_millis(now_ms),
        approved_by: ApprovedBy::Human,
        request_id: Some(request_id.to_owned()),
    };
    store.record_trust(entry.clone());
    ConsentApprovalResult {
        ok: true,
        error: None,
        entry: Some(entry),
    }
}

pub fn reject_consent_plan(store: &mut ConsentStore, request_id: &str) -> ConsentApprovalResult {
    let Some(request) = store.read_pending(request_id) else {
        return consent_error(format!("request not found: {request_id}"));
    };
    if request.status != ConsentStatus::Pending {
        return consent_error(format!(
            "request is {}, cannot reject",
            consent_status_str(request.status)
        ));
    }
    store.update_status(request_id, ConsentStatus::Rejected);
    ConsentApprovalResult {
        ok: true,
        error: None,
        entry: None,
    }
}

#[must_use]
pub fn build_from_sign_payload(
    from: &str,
    timestamp: i64,
    method: &str,
    path: &str,
    body_hash: &str,
) -> String {
    format!(
        "{}:{path}:{timestamp}:{body_hash}:{from}",
        method.to_uppercase()
    )
}

#[must_use]
pub fn build_legacy_from_sign_payload(
    from: &str,
    signed_at: &str,
    method: &str,
    path: &str,
    body_hash: &str,
) -> String {
    format!(
        "{from}\n{signed_at}\n{}\n{path}\n{body_hash}",
        method.to_uppercase()
    )
}

#[must_use]
pub fn verify_hmac_sig(secret: &str, payload: &str, signature_hex: &str) -> bool {
    if signature_hex.is_empty() || !signature_hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return false;
    }
    let expected = hmac_sha256_hex(secret, payload);
    expected.len() == signature_hex.len()
        && constant_time_eq(expected.as_bytes(), signature_hex.as_bytes())
}

#[must_use]
pub fn sign_auto_pair_proof(identity: &AutoPairIdentity, federation_token: &str) -> String {
    hmac_sha256_hex(federation_token, &canonical_auto_pair_identity(identity))
}

#[must_use]
pub fn verify_auto_pair_proof(
    identity: &AutoPairIdentity,
    federation_token: &str,
    proof: &str,
) -> bool {
    if proof.len() != 64 || !proof.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return false;
    }
    let expected = sign_auto_pair_proof(identity, federation_token);
    constant_time_eq(expected.as_bytes(), proof.as_bytes())
}

fn canonical_auto_pair_identity(identity: &AutoPairIdentity) -> String {
    [
        identity.oracle.as_str(),
        identity.node.as_str(),
        identity.url.as_str(),
        identity.pubkey.as_str(),
    ]
    .join("\n")
}

struct SignedInput {
    from: String,
    v3_sig: String,
    v3_timestamp: String,
    legacy_sig: String,
    legacy_signed_at: String,
    has_v3_sig: bool,
    signed: bool,
}

fn signed_input(headers: &Headers) -> SignedInput {
    let from = headers
        .get("x-maw-from")
        .unwrap_or_default()
        .trim()
        .to_owned();
    let v3_sig = headers
        .get("x-maw-signature-v3")
        .unwrap_or_default()
        .trim()
        .to_owned();
    let v3_timestamp = headers
        .get("x-maw-timestamp")
        .unwrap_or_default()
        .trim()
        .to_owned();
    let legacy_sig = headers
        .get("x-maw-signature")
        .unwrap_or_default()
        .trim()
        .to_owned();
    let legacy_signed_at = headers
        .get("x-maw-signed-at")
        .unwrap_or_default()
        .trim()
        .to_owned();
    let has_v3_sig = !from.is_empty() && !v3_sig.is_empty() && !v3_timestamp.is_empty();
    let has_legacy_sig = !from.is_empty() && !legacy_sig.is_empty() && !legacy_signed_at.is_empty();
    SignedInput {
        from,
        v3_sig,
        v3_timestamp,
        legacy_sig,
        legacy_signed_at,
        has_v3_sig,
        signed: has_v3_sig || has_legacy_sig,
    }
}

fn bootstrap_decision(cached: Option<&str>, signed: &SignedInput) -> Option<FromVerifyDecision> {
    match (cached, signed.signed) {
        (None, false) => Some(FromVerifyDecision::AcceptLegacy {
            reason: "no-cache-no-sig".to_owned(),
        }),
        (None, true) => Some(FromVerifyDecision::AcceptTofuRecord {
            reason: "no-cache-signed".to_owned(),
            from: signed.from.clone(),
        }),
        (Some(_), false) => Some(FromVerifyDecision::RefuseUnsigned {
            reason: "cache-no-sig".to_owned(),
            from: (!signed.from.is_empty()).then_some(signed.from.clone()),
        }),
        (Some(_), true) => None,
    }
}

fn signed_at_seconds(signed: &SignedInput) -> Option<i64> {
    if signed.has_v3_sig {
        parse_unix_seconds(&signed.v3_timestamp)
    } else {
        parse_iso_seconds(&signed.legacy_signed_at)
    }
}

#[must_use]
pub fn verify_request(args: &VerifyRequestArgs) -> FromVerifyDecision {
    let signed = signed_input(&args.headers);
    let cached = args
        .cached_pubkey
        .as_deref()
        .filter(|value| !value.is_empty());
    if let Some(decision) = bootstrap_decision(cached, &signed) {
        return decision;
    }

    if signed.from.is_empty() {
        return malformed("missing-from");
    }
    if signed.v3_sig.is_empty() && signed.legacy_sig.is_empty() {
        return malformed("missing-signature");
    }

    let Some(signed_at_sec) = signed_at_seconds(&signed) else {
        return malformed(if signed.has_v3_sig {
            "invalid-timestamp"
        } else {
            "invalid-signed-at"
        });
    };
    let delta = (args.now - signed_at_sec).abs();
    if delta > WINDOW_SEC {
        return FromVerifyDecision::RefuseSkew {
            reason: "timestamp-out-of-window".to_owned(),
            from: signed.from,
            delta,
        };
    }

    let body_hash = hash_body(args.body.as_deref());
    let payload = if signed.has_v3_sig {
        build_from_sign_payload(
            &signed.from,
            signed_at_sec,
            &args.method,
            &args.path,
            &body_hash,
        )
    } else {
        build_legacy_from_sign_payload(
            &signed.from,
            &signed.legacy_signed_at,
            &args.method,
            &args.path,
            &body_hash,
        )
    };
    let signature = if signed.has_v3_sig {
        &signed.v3_sig
    } else {
        &signed.legacy_sig
    };
    let Some(cached) = cached else {
        return malformed("missing-cache");
    };
    if !verify_hmac_sig(cached, &payload, signature) {
        return FromVerifyDecision::RefuseMismatch {
            reason: "signature-invalid".to_owned(),
            from: signed.from,
        };
    }
    FromVerifyDecision::AcceptVerified {
        reason: "cache-sig-valid".to_owned(),
        from: signed.from,
    }
}

#[must_use]
pub fn is_refuse_decision(decision: &FromVerifyDecision) -> bool {
    decision.kind().starts_with("refuse-")
}

fn malformed(reason: &str) -> FromVerifyDecision {
    FromVerifyDecision::RefuseMalformed {
        reason: reason.to_owned(),
    }
}

fn parse_unix_seconds(raw: &str) -> Option<i64> {
    if raw.is_empty() || !raw.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    raw.parse().ok()
}

fn consent_error(error: impl Into<String>) -> ConsentApprovalResult {
    ConsentApprovalResult {
        ok: false,
        error: Some(error.into()),
        entry: None,
    }
}

fn consent_status_str(status: ConsentStatus) -> &'static str {
    match status {
        ConsentStatus::Pending => "pending",
        ConsentStatus::Approved => "approved",
        ConsentStatus::Rejected => "rejected",
        ConsentStatus::Expired => "expired",
    }
}

fn iso_from_unix_millis(ms: i64) -> String {
    let seconds = ms.div_euclid(1_000);
    let millis = ms.rem_euclid(1_000);
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    (
        year,
        u32::try_from(month).expect("civil month fits u32"),
        u32::try_from(day).expect("civil day fits u32"),
    )
}

fn parse_iso_seconds(iso: &str) -> Option<i64> {
    parse_iso_millis(iso).map(|millis| millis / 1_000)
}

fn parse_iso_millis(iso: &str) -> Option<i64> {
    let (date, time) = iso.split_once('T')?;
    let time = time.strip_suffix('Z').unwrap_or(time);
    let mut date_parts = date.split('-');
    let year = date_parts.next()?.parse::<i32>().ok()?;
    let month = date_parts.next()?.parse::<u32>().ok()?;
    let day = date_parts.next()?.parse::<u32>().ok()?;
    let mut time_parts = time.split(':');
    let hour = time_parts.next()?.parse::<u32>().ok()?;
    let minute = time_parts.next()?.parse::<u32>().ok()?;
    let sec_part = time_parts.next()?;
    let (second, millis) = parse_second_millis(sec_part)?;
    if date_parts.next().is_some() || time_parts.next().is_some() {
        return None;
    }
    timestamp_seconds(year, month, day, hour, minute, second).map(|seconds| {
        seconds
            .saturating_mul(1_000)
            .saturating_add(i64::from(millis))
    })
}

fn parse_second_millis(sec_part: &str) -> Option<(u32, u16)> {
    let (second, fraction) = sec_part.split_once('.').unwrap_or((sec_part, ""));
    let second = second.parse::<u32>().ok()?;
    let mut value = 0_u16;
    let mut count = 0_u8;
    for ch in fraction.chars().take(3) {
        let digit = u16::try_from(ch.to_digit(10)?).ok()?;
        value = (value * 10) + digit;
        count += 1;
    }
    let millis = match count {
        0 => 0,
        1 => value * 100,
        2 => value * 10,
        _ => value,
    };
    Some((second, millis))
}

fn timestamp_seconds(
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> Option<i64> {
    if !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => return None,
    };
    if day == 0 || day > max_day {
        return None;
    }
    let year = i64::from(year) - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = i64::from(month);
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(
        (era * 146_097 + doe - 719_468) * 86_400
            + i64::from(hour) * 3_600
            + i64::from(minute) * 60
            + i64::from(second),
    )
}

fn hmac_sha256_hex(secret: &str, payload: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(payload.as_bytes());
    hex_lower(&mac.finalize().into_bytes())
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0_u8;
    for (&left, &right) in a.iter().zip(b) {
        diff |= left ^ right;
    }
    diff == 0
}
