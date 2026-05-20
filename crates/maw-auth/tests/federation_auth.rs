use maw_auth::{
    build_from_sign_payload, build_legacy_from_sign_payload, hash_body, is_loopback,
    is_refuse_decision, resolve_from_address, sign, sign_headers_at, sign_headers_v3_at,
    sign_hmac_sig, sign_request_v3, verify, verify_hmac_sig, verify_request, FromAddressConfig,
    FromVerifyDecision, Headers, VerifyRequestArgs, DEFAULT_ORACLE,
};

const TOKEN: &str = "0123456789abcdef-federation-token";
const PEER_KEY: &str = "feedfacefeedfacefeedfacefeedfacefeedfacefeedfacefeedfacefeedface";
const FROM: &str = "mawjs:m5";
const NOW: i64 = 1_700_000_000;

fn direct_hmac(secret: &str, payload: &str) -> String {
    // sign() includes maw's colon payload shape, so use verify_hmac_sig round-trip
    // by deriving the expected from the implementation under test's public helper.
    let sig = sign_hmac_sig(secret, payload);
    assert_eq!(sig, maw_auth_private_hmac_for_tests(secret, payload));
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

// Ported from maw-js `test/pair-api-default.test.ts` generate/probe cases
// and `src/api/pair.ts` route decision behavior.
#[test]
fn pair_api_generate_and_probe_plans_match_maw_js_status_contract() {
    use maw_auth::{pair_api_generate_plan, pair_api_probe_plan, PairApiConfig, PairCodeStore};

    let config = PairApiConfig {
        node: "node-a".to_owned(),
        oracle: "oracle-a".to_owned(),
        port: 4567,
        base_url: "http://localhost:4567".to_owned(),
        federation_token: "token-a".to_owned(),
        pubkey: "p".repeat(64),
    };
    let mut store = PairCodeStore::default();

    let generated = pair_api_generate_plan(&mut store, &config, "ABC234", None, None, 1_000_000);
    assert_eq!(generated.status, 201);
    assert!(generated.ok);
    assert_eq!(generated.code, "ABC-234");
    assert_eq!(generated.ttl_ms, 120_000);
    assert_eq!(generated.expires_at, 1_120_000);
    assert_eq!(generated.node, "node-a");
    assert_eq!(generated.port, 4567);

    let generated = pair_api_generate_plan(&mut store, &config, "ABC234", Some(5), None, 1_000_000);
    assert_eq!(generated.ttl_ms, 5_000);
    assert_eq!(generated.expires_at, 1_005_000);

    let generated =
        pair_api_generate_plan(&mut store, &config, "ABC234", None, Some(42), 1_000_000);
    assert_eq!(generated.ttl_ms, 42);
    assert_eq!(generated.expires_at, 1_000_042);

    let invalid = pair_api_probe_plan(&store, &config, "bad", 1_000_000);
    assert_eq!(invalid.status, 400);
    assert_eq!(invalid.error.as_deref(), Some("invalid_shape"));

    let missing = pair_api_probe_plan(&store, &config, "ZZZ999", 1_000_000);
    assert_eq!(missing.status, 404);
    assert_eq!(missing.error.as_deref(), Some("not_found"));

    let _ = store.register_at("DEF456", 1, 0);
    let expired = pair_api_probe_plan(&store, &config, "DEF456", 1_000_000);
    assert_eq!(expired.status, 410);
    assert_eq!(expired.error.as_deref(), Some("expired"));

    let live = pair_api_probe_plan(&store, &config, "ABC234", 1_000_000);
    assert_eq!(live.status, 200);
    assert!(live.ok);
    assert_eq!(live.node.as_deref(), Some("node-a"));
}

// Ported from maw-js `test/pair-api-default.test.ts` pair POST/status cases
// and `src/api/pair.ts` consumed-code behavior.
#[test]
fn pair_api_accept_and_status_plans_match_maw_js_consumed_contract() {
    use maw_auth::{
        pair_api_accept_plan, pair_api_status_plan, PairAcceptInput, PairApiConfig, PairCodeStore,
    };

    let config = PairApiConfig {
        node: "node-a".to_owned(),
        oracle: "oracle-a".to_owned(),
        port: 4567,
        base_url: "http://localhost:4567".to_owned(),
        federation_token: "ab".repeat(32),
        pubkey: "p".repeat(64),
    };
    let mut store = PairCodeStore::default();

    let invalid = pair_api_accept_plan(&mut store, &config, "bad", None, 1_000_000);
    assert_eq!(invalid.status, 400);
    assert_eq!(invalid.error.as_deref(), Some("invalid_shape"));

    let bad = pair_api_accept_plan(
        &mut store,
        &config,
        "ABC234",
        Some(PairAcceptInput {
            node: "remote".to_owned(),
            url: None,
        }),
        1_000_000,
    );
    assert_eq!(bad.status, 400);
    assert_eq!(bad.error.as_deref(), Some("bad_request"));

    let missing = pair_api_accept_plan(
        &mut store,
        &config,
        "ABC234",
        Some(PairAcceptInput {
            node: "remote".to_owned(),
            url: Some("http://remote".to_owned()),
        }),
        1_000_000,
    );
    assert_eq!(missing.status, 404);
    assert_eq!(missing.error.as_deref(), Some("not_found"));

    let _ = store.register_at("DEF456", 1, 0);
    let expired = pair_api_accept_plan(
        &mut store,
        &config,
        "DEF456",
        Some(PairAcceptInput {
            node: "remote".to_owned(),
            url: Some("http://remote".to_owned()),
        }),
        1_000_000,
    );
    assert_eq!(expired.status, 410);
    assert_eq!(expired.error.as_deref(), Some("expired"));

    let _ = store.register_at("ABC234", 1_000_000, 1_000_000);
    let accepted = pair_api_accept_plan(
        &mut store,
        &config,
        "ABC234",
        Some(PairAcceptInput {
            node: "remote".to_owned(),
            url: Some("http://remote".to_owned()),
        }),
        1_000_000,
    );
    assert_eq!(accepted.status, 200);
    assert!(accepted.ok);
    assert_eq!(accepted.node.as_deref(), Some("node-a"));
    assert_eq!(accepted.url.as_deref(), Some("http://localhost:4567"));
    assert_eq!(
        accepted.federation_token.as_deref(),
        Some("abababababababababababababababababababababababababababababababab")
    );

    let consumed = pair_api_status_plan(&store, "ABC-234", 1_000_000);
    assert_eq!(consumed.status, 200);
    assert!(consumed.ok);
    assert_eq!(consumed.consumed, Some(true));
    assert_eq!(consumed.remote_node.as_deref(), Some("remote"));
    assert_eq!(consumed.remote_url.as_deref(), Some("http://remote"));

    let not_found = pair_api_status_plan(&store, "ZZZ999", 1_000_000);
    assert_eq!(not_found.status, 404);
    assert_eq!(not_found.error.as_deref(), Some("not_found"));

    let _ = store.register_at("GHJ789", 1, 0);
    let expired_status = pair_api_status_plan(&store, "GHJ789", 1_000_000);
    assert_eq!(expired_status.status, 410);
    assert_eq!(expired_status.error.as_deref(), Some("expired"));

    let _ = store.register_at("JKL234", 1_000_000, 1_000_000);
    let pending = pair_api_status_plan(&store, "JKL234", 1_000_000);
    assert_eq!(pending.status, 200);
    assert_eq!(pending.consumed, Some(false));
}

// Ported from maw-js `test/pair-api-default.test.ts` auto-pair cases
// and `src/api/pair.ts` hello freshness / add-result behavior.
fn pair_api_test_config() -> maw_auth::PairApiConfig {
    maw_auth::PairApiConfig {
        node: "node-a".to_owned(),
        oracle: "oracle-a".to_owned(),
        port: 4567,
        base_url: "http://localhost:4567".to_owned(),
        federation_token: "token-a".to_owned(),
        pubkey: "p".repeat(64),
    }
}

fn auto_pair_input(zid: &str) -> maw_auth::AutoPairInput {
    maw_auth::AutoPairInput {
        node: "remote".to_owned(),
        oracle: None,
        url: "http://remote".to_owned(),
        zid: zid.to_owned(),
        pubkey: None,
    }
}

#[test]
fn pair_api_auto_plan_rejects_missing_stale_and_add_error_cases() {
    use maw_auth::{pair_api_auto_plan, AutoPairAddOutcome, RecentHelloStore};

    let config = pair_api_test_config();
    let mut hellos = RecentHelloStore::default();

    let missing_fields = pair_api_auto_plan(
        &config,
        &hellos,
        None,
        AutoPairAddOutcome::Ok { one_way: true },
        70_001,
    );
    assert_eq!(missing_fields.status, 400);
    assert_eq!(missing_fields.error.as_deref(), Some("missing_fields"));

    let missing = pair_api_auto_plan(
        &config,
        &hellos,
        Some(auto_pair_input("missing")),
        AutoPairAddOutcome::Ok { one_way: true },
        70_001,
    );
    assert_eq!(missing.status, 403);
    assert_eq!(missing.error.as_deref(), Some("no_recent_hello"));

    hellos.record("old", 0);
    hellos.record("fresh", 70_001);
    let stale = pair_api_auto_plan(
        &config,
        &hellos,
        Some(auto_pair_input("old")),
        AutoPairAddOutcome::Ok { one_way: true },
        70_001,
    );
    assert_eq!(stale.status, 403);
    assert_eq!(stale.error.as_deref(), Some("no_recent_hello"));

    hellos.record("mismatch", 70_001);
    let mismatch = pair_api_auto_plan(
        &config,
        &hellos,
        Some(auto_pair_input("mismatch")),
        AutoPairAddOutcome::PubkeyMismatch("key mismatch".to_owned()),
        70_001,
    );
    assert_eq!(mismatch.status, 409);
    assert_eq!(mismatch.error.as_deref(), Some("key mismatch"));

    hellos.record("throws", 70_001);
    let add_error = pair_api_auto_plan(
        &config,
        &hellos,
        Some(auto_pair_input("throws")),
        AutoPairAddOutcome::Error("bad peer".to_owned()),
        70_001,
    );
    assert_eq!(add_error.status, 400);
    assert_eq!(add_error.error.as_deref(), Some("bad peer"));
}

#[test]
fn pair_api_auto_plan_returns_signed_identity_and_peer_add_plan() {
    use maw_auth::{pair_api_auto_plan, AutoPairAddOutcome, AutoPairInput, RecentHelloStore};

    let config = pair_api_test_config();
    let mut hellos = RecentHelloStore::default();
    hellos.record("success", 70_001);

    let success = pair_api_auto_plan(
        &config,
        &hellos,
        Some(AutoPairInput {
            node: "remote".to_owned(),
            oracle: Some("remote-oracle".to_owned()),
            url: "http://remote".to_owned(),
            zid: "success".to_owned(),
            pubkey: Some("r".repeat(64)),
        }),
        AutoPairAddOutcome::Ok { one_way: false },
        70_001,
    );
    assert_eq!(success.status, 200);
    assert!(success.ok);
    assert_eq!(success.node.as_deref(), Some("node-a"));
    assert_eq!(success.oracle.as_deref(), Some("oracle-a"));
    assert_eq!(success.url.as_deref(), Some("http://localhost:4567"));
    assert_eq!(success.pubkey.as_deref(), Some(&"p".repeat(64)[..]));
    assert_eq!(success.one_way, Some(false));
    assert_eq!(success.add_alias.as_deref(), Some("remote"));
    assert_eq!(success.add_url.as_deref(), Some("http://remote"));
    assert_eq!(success.add_node.as_deref(), Some("remote"));
    assert_eq!(success.add_pubkey.as_deref(), Some(&"r".repeat(64)[..]));
    assert_eq!(
        success.add_identity_oracle.as_deref(),
        Some("remote-oracle")
    );
    assert_eq!(success.add_identity_node.as_deref(), Some("remote"));
    assert!(success.mark_symmetric_check);
    assert_eq!(
        success.proof.as_deref(),
        Some("95e63fc871ab14ce17c14e0046cd41b9dd305c086f1ed325fd2c5e62e6ee849f")
    );
}
