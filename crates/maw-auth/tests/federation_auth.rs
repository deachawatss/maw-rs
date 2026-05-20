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
