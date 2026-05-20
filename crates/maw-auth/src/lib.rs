//! Federation auth pure helpers ported from maw-js `src/lib/federation-auth.ts`.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

type HmacSha256 = Hmac<Sha256>;

pub const WINDOW_SEC: i64 = 300;
pub const DEFAULT_ORACLE: &str = "mawjs";

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

fn parse_iso_seconds(iso: &str) -> Option<i64> {
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
    let second = sec_part.split('.').next()?.parse::<u32>().ok()?;
    if date_parts.next().is_some() || time_parts.next().is_some() {
        return None;
    }
    timestamp_seconds(year, month, day, hour, minute, second)
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
