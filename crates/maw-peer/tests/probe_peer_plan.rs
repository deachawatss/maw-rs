use maw_peer::{
    probe_peer_from_plan, PeerIdentity, ProbeErrorCode, ProbeInfoBody, ProbeInfoOutcome,
    ProbeLastError, ProbeMawHandshake, ProbePeerPlan, ProbePeerResult, ProbeRemoteIdentity,
};

fn at() -> String {
    "2026-05-18T00:00:00.000Z".to_owned()
}

fn dns_error() -> ProbeLastError {
    ProbeLastError {
        code: ProbeErrorCode::Dns,
        message: "getaddrinfo ENOTFOUND missing.local".to_owned(),
        at: at(),
    }
}

#[test]
fn probe_peer_plan_returns_modern_identity_like_maw_js_probe_peer() {
    let result = probe_peer_from_plan(&ProbePeerPlan {
        url: "http://peer.test:3456/some/path".to_owned(),
        now: at(),
        dns_error: None,
        info: ProbeInfoOutcome::Body(ProbeInfoBody {
            maw: ProbeMawHandshake::SchemaObject("1".to_owned()),
            node: Some("peer-node".to_owned()),
            name: None,
            nickname: Some("Peer Nick".to_owned()),
        }),
        identity: Some(ProbeRemoteIdentity::Body {
            pubkey: Some("pub-123".to_owned()),
            oracle: Some("oracle-x".to_owned()),
            node: Some("peer-node".to_owned()),
        }),
    });

    assert_eq!(
        result,
        ProbePeerResult {
            node: Some("peer-node".to_owned()),
            nickname: Some("Peer Nick".to_owned()),
            pubkey: Some("pub-123".to_owned()),
            identity: Some(PeerIdentity {
                oracle: "oracle-x".to_owned(),
                node: "peer-node".to_owned(),
            }),
            error: None,
        }
    );
}

#[test]
fn probe_peer_plan_uses_legacy_name_and_default_oracle_identity() {
    let result = probe_peer_from_plan(&ProbePeerPlan {
        url: "http://127.0.0.1:3456".to_owned(),
        now: at(),
        dns_error: None,
        info: ProbeInfoOutcome::Body(ProbeInfoBody {
            maw: ProbeMawHandshake::LegacyTrue,
            node: None,
            name: Some("legacy-name".to_owned()),
            nickname: Some(String::new()),
        }),
        identity: Some(ProbeRemoteIdentity::Body {
            pubkey: Some("pub-default".to_owned()),
            oracle: None,
            node: Some("legacy-name".to_owned()),
        }),
    });

    assert_eq!(
        result,
        ProbePeerResult {
            node: Some("legacy-name".to_owned()),
            nickname: None,
            pubkey: Some("pub-default".to_owned()),
            identity: Some(PeerIdentity {
                oracle: "mawjs".to_owned(),
                node: "legacy-name".to_owned(),
            }),
            error: None,
        }
    );
}

#[test]
fn probe_peer_plan_keeps_info_success_when_identity_is_absent_or_malformed() {
    let base = ProbePeerPlan {
        url: "http://127.0.0.1:3456".to_owned(),
        now: at(),
        dns_error: None,
        info: ProbeInfoOutcome::Body(ProbeInfoBody {
            maw: ProbeMawHandshake::LegacyTrue,
            node: Some("legacy".to_owned()),
            name: None,
            nickname: Some("Legacy Peer".to_owned()),
        }),
        identity: Some(ProbeRemoteIdentity::Missing),
    };

    assert_eq!(
        probe_peer_from_plan(&base),
        ProbePeerResult {
            node: Some("legacy".to_owned()),
            nickname: Some("Legacy Peer".to_owned()),
            pubkey: None,
            identity: None,
            error: None,
        }
    );

    let mut malformed = base;
    malformed.identity = Some(ProbeRemoteIdentity::MalformedJson);
    assert_eq!(
        probe_peer_from_plan(&malformed).node.as_deref(),
        Some("legacy")
    );
    assert_eq!(probe_peer_from_plan(&malformed).identity, None);
}

#[test]
fn probe_peer_plan_returns_structured_failures_like_maw_js_probe_peer() {
    let dns = probe_peer_from_plan(&ProbePeerPlan {
        url: "http://missing.local:3456".to_owned(),
        now: at(),
        dns_error: Some(dns_error()),
        info: ProbeInfoOutcome::Body(ProbeInfoBody {
            maw: ProbeMawHandshake::LegacyTrue,
            node: Some("never-fetched".to_owned()),
            name: None,
            nickname: None,
        }),
        identity: None,
    });
    assert_eq!(dns.node, None);
    assert_eq!(
        dns.error.as_ref().map(|err| err.code),
        Some(ProbeErrorCode::Dns)
    );
    assert_eq!(
        dns.error.as_ref().map(|err| err.message.as_str()),
        Some("getaddrinfo ENOTFOUND missing.local")
    );

    let http = probe_peer_from_plan(&ProbePeerPlan {
        url: "http://127.0.0.1:3456".to_owned(),
        now: at(),
        dns_error: None,
        info: ProbeInfoOutcome::HttpStatus {
            status: 503,
            ok: false,
        },
        identity: None,
    });
    assert_eq!(
        http.error.as_ref().map(|err| err.code),
        Some(ProbeErrorCode::Http5xx)
    );
    assert_eq!(
        http.error.as_ref().map(|err| err.message.as_str()),
        Some("HTTP 503 from http://127.0.0.1:3456/info")
    );

    let invalid_json = probe_peer_from_plan(&ProbePeerPlan {
        url: "http://127.0.0.1:3456".to_owned(),
        now: at(),
        dns_error: None,
        info: ProbeInfoOutcome::InvalidJson,
        identity: None,
    });
    assert_eq!(
        invalid_json.error.as_ref().map(|err| err.code),
        Some(ProbeErrorCode::BadBody)
    );
    assert_eq!(
        invalid_json.error.as_ref().map(|err| err.message.as_str()),
        Some("/info body was not valid JSON")
    );

    let missing_maw = probe_peer_from_plan(&ProbePeerPlan {
        url: "http://127.0.0.1:3456".to_owned(),
        now: at(),
        dns_error: None,
        info: ProbeInfoOutcome::Body(ProbeInfoBody {
            maw: ProbeMawHandshake::Missing,
            node: Some("not-maw".to_owned()),
            name: None,
            nickname: None,
        }),
        identity: None,
    });
    assert_eq!(
        missing_maw.error.as_ref().map(|err| err.code),
        Some(ProbeErrorCode::BadBody)
    );
    assert_eq!(
        missing_maw.error.as_ref().map(|err| err.message.as_str()),
        Some("/info response missing valid \"maw\" handshake field")
    );

    let nameless = probe_peer_from_plan(&ProbePeerPlan {
        url: "http://127.0.0.1:3456".to_owned(),
        now: at(),
        dns_error: None,
        info: ProbeInfoOutcome::Body(ProbeInfoBody {
            maw: ProbeMawHandshake::LegacyTrue,
            node: None,
            name: None,
            nickname: Some("nameless".to_owned()),
        }),
        identity: None,
    });
    assert_eq!(
        nameless.error.as_ref().map(|err| err.code),
        Some(ProbeErrorCode::BadBody)
    );
    assert_eq!(
        nameless.error.as_ref().map(|err| err.message.as_str()),
        Some("/info response had neither \"node\" nor \"name\" string")
    );
}

#[test]
fn probe_peer_plan_classifies_fetch_failures_with_context() {
    let refused = probe_peer_from_plan(&ProbePeerPlan {
        url: "http://127.0.0.1:3456".to_owned(),
        now: at(),
        dns_error: None,
        info: ProbeInfoOutcome::FetchCode {
            code: "ECONNREFUSED".to_owned(),
            message: "connect ECONNREFUSED".to_owned(),
        },
        identity: None,
    });
    assert_eq!(
        refused.error.as_ref().map(|err| err.code),
        Some(ProbeErrorCode::Refused)
    );
    assert_eq!(
        refused.error.as_ref().map(|err| err.message.as_str()),
        Some("connect ECONNREFUSED")
    );

    let tls_non_error_throw = probe_peer_from_plan(&ProbePeerPlan {
        url: "http://127.0.0.1:3456".to_owned(),
        now: at(),
        dns_error: None,
        info: ProbeInfoOutcome::FetchCodeWithoutMessage {
            code: "UNABLE_TO_VERIFY_LEAF_SIGNATURE".to_owned(),
        },
        identity: None,
    });
    assert_eq!(
        tls_non_error_throw.error.as_ref().map(|err| err.code),
        Some(ProbeErrorCode::Tls)
    );
    assert_eq!(
        tls_non_error_throw
            .error
            .as_ref()
            .map(|err| err.message.as_str()),
        Some("fetch http://127.0.0.1:3456/info failed")
    );
}
