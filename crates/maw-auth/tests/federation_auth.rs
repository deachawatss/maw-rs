use maw_auth::{
    build_from_sign_payload, build_legacy_from_sign_payload, hash_body, is_loopback,
    is_refuse_decision, resolve_from_address, sign, sign_headers_at, sign_headers_v3_at,
    sign_request_v3, verify, verify_hmac_sig, verify_request, FromAddressConfig,
    FromVerifyDecision, Headers, VerifyRequestArgs, DEFAULT_ORACLE,
};

const TOKEN: &str = "0123456789abcdef-federation-token";
const PEER_KEY: &str = "feedfacefeedfacefeedfacefeedfacefeedfacefeedfacefeedfacefeedface";
const FROM: &str = "mawjs:m5";
const NOW: i64 = 1_700_000_000;

fn direct_hmac(secret: &str, payload: &str) -> String {
    // sign() includes maw's colon payload shape, so use verify_hmac_sig round-trip
    // by deriving the expected from the implementation under test's public helper.
    let sig = maw_auth_private_hmac_for_tests(secret, payload);
    assert!(verify_hmac_sig(secret, payload, &sig));
    sig
}

fn maw_auth_private_hmac_for_tests(secret: &str, payload: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(payload.as_bytes());
    let bytes = mac.finalize().into_bytes();
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut out, "{byte:02x}").expect("writing to String cannot fail");
    }
    out
}

#[test]
fn hashing_and_signing_helpers_cover_v1_v2_v3_and_validation_branches() {
    assert_eq!(hash_body(None), "");
    assert_eq!(hash_body(Some(b"")), "");
    assert_eq!(hash_body(Some(b"body")).len(), 64);

    let sig = sign(TOKEN, "POST", "/api/send", NOW, "");
    assert!(verify(TOKEN, "POST", "/api/send", NOW, &sig, "", NOW));
    assert!(!verify(
        TOKEN,
        "POST",
        "/api/send",
        NOW - 301,
        &sig,
        "",
        NOW
    ));
    assert!(!verify(TOKEN, "POST", "/api/send", NOW, "short", "", NOW));
    assert!(!verify(
        TOKEN,
        "POST",
        "/api/send",
        NOW,
        &"z".repeat(64),
        "",
        NOW
    ));

    assert!(is_loopback(Some("127.9.0.1")));
    assert!(is_loopback(Some("::1")));
    assert!(is_loopback(Some("localhost")));
    assert!(!is_loopback(None));

    let h1 = sign_headers_at(TOKEN, "GET", "/api/send", None, NOW);
    assert_eq!(h1.get("X-Maw-Auth-Version"), None);
    let h2 = sign_headers_at(TOKEN, "POST", "/api/send", Some(b"body"), NOW);
    assert_eq!(h2.get("X-Maw-Auth-Version"), Some("v2"));

    assert!(sign_request_v3("", FROM, "POST", "/api/send", NOW, None)
        .expect_err("missing peer key should throw")
        .contains("peerKey"));
    assert!(
        sign_request_v3(PEER_KEY, "", "POST", "/api/send", NOW, None)
            .expect_err("missing from address should throw")
            .contains("fromAddress")
    );
    let v3 = sign_request_v3(PEER_KEY, FROM, "post", "/api/send", NOW, Some(b"body"))
        .expect("valid v3 signing should work");
    assert_eq!(
        v3.signature,
        direct_hmac(
            PEER_KEY,
            &build_from_sign_payload(FROM, NOW, "POST", "/api/send", &hash_body(Some(b"body")))
        )
    );
    assert_eq!(
        sign_headers_v3_at(PEER_KEY, FROM, "POST", "/api/send", None, NOW)
            .expect("v3 headers should sign")
            .get("X-Maw-Auth-Version"),
        Some("v3")
    );
    assert_eq!(
        resolve_from_address(&FromAddressConfig {
            oracle: None,
            node: Some("m5".to_owned())
        }),
        Some(format!("{DEFAULT_ORACLE}:m5"))
    );
    assert_eq!(
        resolve_from_address(&FromAddressConfig {
            oracle: Some("pulse".to_owned()),
            node: None
        }),
        None
    );
}

#[test]
fn sign_is_deterministic_and_sensitive_to_payload_fields() {
    let base = sign(TOKEN, "POST", "/api/send", NOW, "");
    assert_eq!(base.len(), 64);
    assert_eq!(base, sign(TOKEN, "POST", "/api/send", NOW, ""));
    assert_ne!(base, sign(TOKEN, "GET", "/api/send", NOW, ""));
    assert_ne!(base, sign(TOKEN, "POST", "/api/talk", NOW, ""));
    assert_ne!(base, sign(TOKEN, "POST", "/api/send", NOW + 1, ""));
    assert_ne!(
        base,
        sign("different-token-also-long", "POST", "/api/send", NOW, "")
    );
}

fn verify_req(headers: Headers, body: &[u8], cached_pubkey: Option<&str>) -> FromVerifyDecision {
    verify_request(&VerifyRequestArgs {
        method: "POST".to_owned(),
        path: "/api/send".to_owned(),
        headers,
        body: Some(body.to_vec()),
        cached_pubkey: cached_pubkey.map(str::to_owned),
        now: NOW,
    })
}

#[test]
fn verify_request_covers_o6_current_v3_decisions_and_malformed_cases() {
    assert_eq!(
        verify_req(Headers::new([] as [(&str, &str); 0]), b"", None),
        FromVerifyDecision::AcceptLegacy {
            reason: "no-cache-no-sig".to_owned()
        }
    );

    let signed = sign_headers_v3_at(PEER_KEY, FROM, "POST", "/api/send", Some(b"body"), NOW)
        .expect("v3 headers should sign");
    assert_eq!(
        verify_req(signed.clone(), b"body", None).kind(),
        "accept-tofu-record"
    );
    assert_eq!(
        verify_req(Headers::new([("x-maw-from", FROM)]), b"", Some(PEER_KEY)),
        FromVerifyDecision::RefuseUnsigned {
            reason: "cache-no-sig".to_owned(),
            from: Some(FROM.to_owned()),
        }
    );
    assert_eq!(
        verify_req(signed.clone(), b"body", Some(PEER_KEY)).kind(),
        "accept-verified"
    );
    assert_eq!(
        verify_req(signed.clone(), b"tampered", Some(PEER_KEY)).kind(),
        "refuse-mismatch"
    );
    assert_eq!(
        verify_req(
            Headers::new([
                ("X-Maw-From", FROM),
                (
                    "X-Maw-Signature-V3",
                    signed.get("x-maw-signature-v3").expect("sig")
                ),
                ("X-Maw-Timestamp", &(NOW - 301).to_string()),
            ]),
            b"body",
            Some(PEER_KEY),
        )
        .kind(),
        "refuse-skew"
    );
    assert_eq!(
        verify_req(
            Headers::new([
                ("x-maw-from", FROM),
                ("x-maw-signature-v3", &"0".repeat(64)),
                ("x-maw-timestamp", "nope"),
            ]),
            b"",
            Some(PEER_KEY),
        ),
        FromVerifyDecision::RefuseMalformed {
            reason: "invalid-timestamp".to_owned()
        }
    );
}

#[test]
fn verify_request_accepts_legacy_from_signing_and_identifies_refusals() {
    let iso = "2023-11-14T22:13:20.000Z";
    let legacy_payload =
        build_legacy_from_sign_payload(FROM, iso, "POST", "/api/send", &hash_body(Some(b"body")));
    let legacy_headers = Headers::new([
        ("x-maw-from", FROM),
        ("x-maw-signature", &direct_hmac(PEER_KEY, &legacy_payload)),
        ("x-maw-signed-at", iso),
    ]);
    let legacy = verify_req(legacy_headers, b"body", Some(PEER_KEY));
    assert_eq!(legacy.kind(), "accept-verified");
    assert!(!is_refuse_decision(&legacy));
    assert!(is_refuse_decision(&FromVerifyDecision::RefuseMismatch {
        reason: "signature-invalid".to_owned(),
        from: FROM.to_owned(),
    }));
}

// Ported from maw-js `test/scout-pair-proof.test.ts` and
// `src/transports/scout-pair-proof.ts`.
#[test]
fn auto_pair_proofs_sign_stable_canonical_identity_fields() {
    use maw_auth::{sign_auto_pair_proof, AutoPairIdentity};

    let identity = AutoPairIdentity {
        node: "m5".to_owned(),
        oracle: "mawjs".to_owned(),
        url: "http://m5.local:3456".to_owned(),
        pubkey: "pub-abc".to_owned(),
    };

    let proof = sign_auto_pair_proof(&identity, "token-a");
    assert_eq!(proof.len(), 64);
    assert!(proof.chars().all(|ch| ch.is_ascii_hexdigit()));
    assert_eq!(sign_auto_pair_proof(&identity.clone(), "token-a"), proof);
    assert_ne!(
        sign_auto_pair_proof(
            &AutoPairIdentity {
                node: "other".to_owned(),
                ..identity
            },
            "token-a",
        ),
        proof
    );
}

// Ported from maw-js `test/scout-pair-proof.test.ts` and
// `src/transports/scout-pair-proof.ts`.
#[test]
fn auto_pair_proofs_verify_valid_proofs_and_reject_wrong_inputs() {
    use maw_auth::{sign_auto_pair_proof, verify_auto_pair_proof, AutoPairIdentity};

    let identity = AutoPairIdentity {
        node: "m5".to_owned(),
        oracle: "mawjs".to_owned(),
        url: "http://m5.local:3456".to_owned(),
        pubkey: "pub-abc".to_owned(),
    };
    let proof = sign_auto_pair_proof(&identity, "token-a");

    assert!(verify_auto_pair_proof(&identity, "token-a", &proof));
    assert!(!verify_auto_pair_proof(&identity, "token-b", &proof));
    assert!(!verify_auto_pair_proof(
        &AutoPairIdentity {
            pubkey: "pub-other".to_owned(),
            ..identity.clone()
        },
        "token-a",
        &proof,
    ));
    assert!(!verify_auto_pair_proof(&identity, "token-a", &proof[2..]));
    assert!(!verify_auto_pair_proof(
        &identity,
        "token-a",
        &"z".repeat(64)
    ));
}

// Ported from maw-js `src/lib/pair-codes.ts` and `test/pair-api-default.test.ts`.
#[test]
fn pair_code_helpers_match_maw_js_shape_format_and_redaction() {
    use maw_auth::{
        generate_pair_code_from_bytes, is_valid_pair_code_shape, normalize_pair_code,
        pretty_pair_code, redact_pair_code, PAIR_CODE_ALPHABET,
    };

    assert_eq!(normalize_pair_code("abc-234"), "ABC234");
    assert_eq!(normalize_pair_code(" ab c-2 34\n"), "ABC234");
    assert!(is_valid_pair_code_shape("ABC-234"));
    assert!(is_valid_pair_code_shape("abc234"));
    assert!(!is_valid_pair_code_shape("ABCDE"));
    assert!(!is_valid_pair_code_shape("ABCDEFG"));
    assert!(!is_valid_pair_code_shape("ABCDE0"));
    assert!(!is_valid_pair_code_shape("ABCDE1"));
    assert!(!is_valid_pair_code_shape("ABCDEI"));
    assert!(!is_valid_pair_code_shape("ABCDEO"));

    assert_eq!(pretty_pair_code("abc234"), "ABC-234");
    assert_eq!(pretty_pair_code("bad"), "BAD");
    assert_eq!(redact_pair_code("abc234"), "ABC-***");
    assert_eq!(redact_pair_code("ab"), "***");

    let code = generate_pair_code_from_bytes(&[0, 1, 31, 32, 33, 255]);
    assert_eq!(code.len(), 6);
    assert!(code.chars().all(|ch| PAIR_CODE_ALPHABET.contains(ch)));
    assert_eq!(code, "AB9AB9");
}

// Ported from maw-js `src/lib/pair-codes.ts` and `test/pair-api-default.test.ts`.
#[test]
fn pair_code_store_register_lookup_and_consume_match_maw_js_ttl_contract() {
    use maw_auth::{LookupResult, PairCodeStore};

    let mut store = PairCodeStore::default();
    let entry = store.register_at("abc-234", 120_000, 1_000_000);
    assert_eq!(entry.code, "ABC234");
    assert_eq!(entry.created_at, 1_000_000);
    assert_eq!(entry.expires_at, 1_120_000);
    assert!(!entry.consumed);

    assert_eq!(
        store.lookup_at("ABC234", 1_000_000),
        LookupResult::Live(entry)
    );
    assert_eq!(store.lookup_at("ZZZ999", 1_000_000), LookupResult::NotFound);
    assert_eq!(store.lookup_at("ABC234", 1_120_001), LookupResult::Expired);

    let consumed = store.consume_at("abc 234", 1_000_001);
    assert!(matches!(consumed, LookupResult::Live(_)));
    assert_eq!(
        store.lookup_at("ABC-234", 1_000_002),
        LookupResult::Consumed
    );
    assert_eq!(
        store.consume_at("ABC234", 1_000_003),
        LookupResult::Consumed
    );
}

// Ported from maw-js `src/core/consent/pin.ts` and
// `test/core/consent/consent.test.ts`.
#[test]
fn consent_pin_hash_and_verify_match_maw_js_normalized_shape_contract() {
    use maw_auth::{hash_consent_pin, verify_consent_pin};

    let h1 = hash_consent_pin("ABC-DEF");
    let h2 = hash_consent_pin("abcdef");
    let h3 = hash_consent_pin("ABCDEF");
    assert_eq!(h1, h2);
    assert_eq!(h2, h3);
    assert_eq!(h1.len(), 64);
    assert!(h1.chars().all(|ch| ch.is_ascii_hexdigit()));

    assert!(verify_consent_pin("ABC-DEF", &h1));
    assert!(verify_consent_pin("abcdef", &h1));
    assert!(!verify_consent_pin("BBBBBB", &h1));
    assert!(!verify_consent_pin("ABCDE", &h1));
    assert!(!verify_consent_pin("ABCDEFG", &h1));
    assert!(!verify_consent_pin("ABCDE0", &h1));
}

// Ported from maw-js `src/core/consent/store.ts` and
// `test/core/consent/consent.test.ts` trust/pending store cases.
#[test]
fn consent_trust_store_matches_maw_js_key_asymmetry_and_sorting() {
    use maw_auth::{trust_key, ApprovedBy, ConsentAction, ConsentStore, TrustEntry};

    assert_eq!(trust_key("a", "b", ConsentAction::Hey), "a→b:hey");

    let mut store = ConsentStore::default();
    assert!(!store.is_trusted("a", "b", ConsentAction::Hey));

    store.record_trust(TrustEntry {
        from: "a".to_owned(),
        to: "b".to_owned(),
        action: ConsentAction::Hey,
        approved_at: "2026-01-02".to_owned(),
        approved_by: ApprovedBy::Human,
        request_id: Some("r1".to_owned()),
    });
    store.record_trust(TrustEntry {
        from: "c".to_owned(),
        to: "d".to_owned(),
        action: ConsentAction::Hey,
        approved_at: "2026-01-01".to_owned(),
        approved_by: ApprovedBy::Human,
        request_id: None,
    });

    assert!(store.is_trusted("a", "b", ConsentAction::Hey));
    assert!(!store.is_trusted("b", "a", ConsentAction::Hey));
    assert!(!store.is_trusted("a", "b", ConsentAction::TeamInvite));
    assert_eq!(
        store
            .list_trust()
            .into_iter()
            .map(|entry| entry.from)
            .collect::<Vec<_>>(),
        vec!["c", "a"]
    );
    assert!(store.remove_trust("a", "b", ConsentAction::Hey));
    assert!(!store.is_trusted("a", "b", ConsentAction::Hey));
    assert!(!store.remove_trust("a", "b", ConsentAction::Hey));
}

// Ported from maw-js `src/core/consent/store.ts` and
// `test/core/consent/consent.test.ts` trust/pending store cases.
#[test]
fn consent_pending_store_matches_maw_js_status_expiry_and_ordering() {
    use maw_auth::{
        apply_consent_expiry, ConsentAction, ConsentStatus, ConsentStore, PendingRequest,
    };

    let pending = PendingRequest {
        id: "abc".to_owned(),
        from: "neo".to_owned(),
        to: "mawjs".to_owned(),
        action: ConsentAction::Hey,
        summary: "test".to_owned(),
        pin_hash: "hash".to_owned(),
        created_at: "2026-01-02T00:00:00.000Z".to_owned(),
        expires_at: "2026-01-02T00:01:00.000Z".to_owned(),
        status: ConsentStatus::Pending,
    };
    assert_eq!(
        apply_consent_expiry(&pending, 1_767_312_061_000).status,
        ConsentStatus::Expired
    );
    assert_eq!(
        apply_consent_expiry(
            &PendingRequest {
                status: ConsentStatus::Approved,
                ..pending.clone()
            },
            1_767_312_061_000
        )
        .status,
        ConsentStatus::Approved
    );

    let mut store = ConsentStore::default();
    store.write_pending(pending.clone());
    store.write_pending(PendingRequest {
        id: "newer".to_owned(),
        created_at: "2026-01-03T00:00:00.000Z".to_owned(),
        ..pending.clone()
    });

    assert_eq!(store.read_pending("abc").expect("pending").id, "abc");
    assert_eq!(
        store
            .list_pending()
            .into_iter()
            .map(|req| req.id)
            .collect::<Vec<_>>(),
        vec!["newer", "abc"]
    );
    assert!(store.update_status("abc", ConsentStatus::Rejected));
    assert_eq!(
        store.read_pending("abc").expect("updated").status,
        ConsentStatus::Rejected
    );
    assert!(!store.update_status("missing", ConsentStatus::Approved));
    assert!(store.delete_pending("abc"));
    assert!(store.read_pending("abc").is_none());
    assert!(!store.delete_pending("abc"));
}

// Ported from maw-js `src/core/consent/request.ts` and
// `test/core/consent/consent.test.ts` request/approve/reject cases.
#[test]
fn consent_request_plan_mirrors_pending_and_models_peer_post_failures() {
    use maw_auth::{
        request_consent_plan, ConsentAction, ConsentRequestArgs, ConsentStore, PeerPostResult,
    };

    let mut store = ConsentStore::default();
    let ok = request_consent_plan(
        &mut store,
        ConsentRequestArgs {
            from: "neo".to_owned(),
            to: "mawjs".to_owned(),
            action: ConsentAction::Hey,
            summary: "hello".to_owned(),
            peer_url: None,
            request_id: "00112233445566778899aabb".to_owned(),
            pin: "ABCDEF".to_owned(),
            now_ms: 1_767_312_000_000,
            peer_post: PeerPostResult::Skipped,
        },
    );
    assert!(ok.ok);
    assert_eq!(ok.pin.as_deref(), Some("ABCDEF"));
    assert_eq!(ok.request_id.as_deref(), Some("00112233445566778899aabb"));
    assert_eq!(
        store
            .read_pending("00112233445566778899aabb")
            .expect("pending")
            .summary,
        "hello"
    );

    let mut store = ConsentStore::default();
    let posted = request_consent_plan(
        &mut store,
        ConsentRequestArgs {
            from: "neo".to_owned(),
            to: "mawjs".to_owned(),
            action: ConsentAction::Hey,
            summary: "hi".to_owned(),
            peer_url: Some("http://peer:3456".to_owned()),
            request_id: "req-http".to_owned(),
            pin: "ABCDEF".to_owned(),
            now_ms: 1_767_312_000_000,
            peer_post: PeerPostResult::HttpStatus(500),
        },
    );
    assert!(!posted.ok);
    assert_eq!(
        posted.peer_url.as_deref(),
        Some("http://peer:3456/api/consent/request")
    );
    assert_eq!(posted.peer_method.as_deref(), Some("POST"));
    assert!(posted.peer_body.as_ref().expect("body").pin.is_none());
    assert!(posted.error.as_deref().expect("error").contains("500"));
    assert!(store.read_pending("req-http").is_some());

    let network = request_consent_plan(
        &mut store,
        ConsentRequestArgs {
            from: "neo".to_owned(),
            to: "mawjs".to_owned(),
            action: ConsentAction::Hey,
            summary: "hi".to_owned(),
            peer_url: Some("http://peer:3456".to_owned()),
            request_id: "req-network".to_owned(),
            pin: "ABCDEF".to_owned(),
            now_ms: 1_767_312_000_000,
            peer_post: PeerPostResult::NetworkError("ECONNREFUSED".to_owned()),
        },
    );
    assert!(!network.ok);
    assert!(network
        .error
        .as_deref()
        .expect("error")
        .contains("ECONNREFUSED"));
}

// Ported from maw-js `src/core/consent/request.ts` and
// `test/core/consent/consent.test.ts` request/approve/reject cases.
#[test]
fn consent_approve_and_reject_plans_match_maw_js_state_machine() {
    use maw_auth::{
        approve_consent_plan, reject_consent_plan, request_consent_plan, ConsentAction,
        ConsentRequestArgs, ConsentStatus, ConsentStore, PeerPostResult,
    };

    let mut store = ConsentStore::default();
    let req = ConsentRequestArgs {
        from: "neo".to_owned(),
        to: "mawjs".to_owned(),
        action: ConsentAction::Hey,
        summary: "x".to_owned(),
        peer_url: None,
        request_id: "req-ok".to_owned(),
        pin: "ABCDEF".to_owned(),
        now_ms: 1_767_312_000_000,
        peer_post: PeerPostResult::Skipped,
    };
    request_consent_plan(&mut store, req);
    let approved = approve_consent_plan(&mut store, "req-ok", "ABCDEF", 1_767_312_001_000);
    assert!(approved.ok);
    assert_eq!(approved.entry.as_ref().expect("entry").from, "neo");
    assert!(store.is_trusted("neo", "mawjs", ConsentAction::Hey));
    assert_eq!(
        store.read_pending("req-ok").expect("pending").status,
        ConsentStatus::Approved
    );

    assert!(!approve_consent_plan(&mut store, "missing", "ABCDEF", 1_767_312_001_000).ok);
    let second = approve_consent_plan(&mut store, "req-ok", "ABCDEF", 1_767_312_001_000);
    assert!(!second.ok);
    assert!(second.error.as_deref().expect("error").contains("approved"));

    let mut store = ConsentStore::default();
    request_consent_plan(
        &mut store,
        ConsentRequestArgs {
            request_id: "req-bad-pin".to_owned(),
            from: "a".to_owned(),
            to: "b".to_owned(),
            action: ConsentAction::Hey,
            summary: "x".to_owned(),
            peer_url: None,
            pin: "ABCDEF".to_owned(),
            now_ms: 1_767_312_000_000,
            peer_post: PeerPostResult::Skipped,
        },
    );
    let bad_pin = approve_consent_plan(&mut store, "req-bad-pin", "ZZZZZZ", 1_767_312_001_000);
    assert!(!bad_pin.ok);
    assert!(bad_pin.error.as_deref().expect("error").contains("PIN"));
    assert!(!store.is_trusted("a", "b", ConsentAction::Hey));

    let rejected = reject_consent_plan(&mut store, "req-bad-pin");
    assert!(rejected.ok);
    assert_eq!(
        store.read_pending("req-bad-pin").expect("pending").status,
        ConsentStatus::Rejected
    );
    assert!(!store.is_trusted("a", "b", ConsentAction::Hey));
}

// Ported from maw-js `src/core/consent/request.ts` and
// `test/core/consent/consent.test.ts` newRequestId cases.
#[test]
fn consent_request_id_generation_matches_maw_js_24_hex_contract() {
    use maw_auth::consent_request_id_from_bytes;

    let id = consent_request_id_from_bytes(&[0, 1, 2, 10, 15, 16, 31, 127, 128, 200, 254, 255]);
    assert_eq!(id, "0001020a0f101f7f80c8feff");
    assert_eq!(id.len(), 24);
    assert!(id
        .chars()
        .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase()));
    assert_eq!(
        consent_request_id_from_bytes(&[0xab; 20]),
        "abababababababababababab"
    );
}
