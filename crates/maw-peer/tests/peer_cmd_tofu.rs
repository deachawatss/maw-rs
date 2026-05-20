use maw_peer::{
    cmd_peer_add_from_plan, cmd_peer_probe_from_plan, PeerAddPlan, PeerIdentity, PeerProbePlan,
    PeerRecord, ProbeErrorCode, ProbeLastError, ProbePeerResult,
};
use std::collections::BTreeMap;

fn peer(url: &str) -> PeerRecord {
    PeerRecord {
        url: url.to_owned(),
        node: None,
        added_at: "2026-05-18T00:00:00.000Z".to_owned(),
        last_seen: None,
        last_error: None,
        nickname: None,
        pubkey: None,
        pubkey_first_seen: None,
        identity: None,
        one_way: None,
        last_symmetric_check: None,
    }
}

fn ok_probe(node: &str, pubkey: Option<&str>) -> ProbePeerResult {
    ProbePeerResult {
        node: Some(node.to_owned()),
        nickname: None,
        pubkey: pubkey.map(str::to_owned),
        identity: Some(PeerIdentity {
            oracle: "remote-oracle".to_owned(),
            node: node.to_owned(),
        }),
        error: None,
    }
}

#[test]
fn cmd_peer_add_refuses_tofu_mismatch_before_overwriting_existing_peer() {
    let mut existing = peer("http://old-frank");
    existing.node = Some("old-node".to_owned());
    existing.pubkey = Some("cached-key".to_owned());
    existing.last_seen = Some("old-seen".to_owned());
    let peers = BTreeMap::from([("frank".to_owned(), existing.clone())]);

    let result = cmd_peer_add_from_plan(&PeerAddPlan {
        alias: "frank".to_owned(),
        url: "http://new-frank".to_owned(),
        node: None,
        authenticated_pubkey: Some("auth-pubkey".to_owned()),
        authenticated_identity: Some(PeerIdentity {
            oracle: "bob-oracle".to_owned(),
            node: "bob-node".to_owned(),
        }),
        mark_symmetric_check: true,
        one_way: Some(false),
        now: "2026-05-18T12:00:00.000Z".to_owned(),
        peers,
        probe: ok_probe("new-node", Some("observed-key")),
    })
    .unwrap();

    assert_eq!(result.alias, "frank");
    assert!(result.overwrote);
    assert_eq!(result.peer.url, "http://old-frank");
    assert_eq!(result.peer.node.as_deref(), Some("old-node"));
    assert!(result.probe_error.is_none());
    assert!(result
        .pubkey_mismatch
        .as_ref()
        .unwrap()
        .to_string()
        .contains("maw peers forget frank"));
    assert_eq!(result.peers_after["frank"], existing);
}

#[test]
fn cmd_peer_add_bootstraps_pubkey_identity_probe_error_and_preserves_cached_pin_on_readd() {
    let probe_error = ProbeLastError {
        code: ProbeErrorCode::Timeout,
        message: "slow".to_owned(),
        at: "2026-05-18T00:00:00.000Z".to_owned(),
    };
    let result = cmd_peer_add_from_plan(&PeerAddPlan {
        alias: "bob".to_owned(),
        url: "https://bob.example".to_owned(),
        node: Some("operator-node".to_owned()),
        authenticated_pubkey: Some("auth-pubkey".to_owned()),
        authenticated_identity: Some(PeerIdentity {
            oracle: "bob-oracle".to_owned(),
            node: "bob-node".to_owned(),
        }),
        mark_symmetric_check: true,
        one_way: Some(false),
        now: "2026-05-18T12:00:00.000Z".to_owned(),
        peers: BTreeMap::new(),
        probe: ProbePeerResult {
            node: None,
            nickname: Some("bobby".to_owned()),
            pubkey: None,
            identity: None,
            error: Some(probe_error.clone()),
        },
    })
    .unwrap();

    assert!(!result.overwrote);
    assert_eq!(result.probe_error, Some(probe_error.clone()));
    assert_eq!(result.peer.url, "https://bob.example");
    assert_eq!(result.peer.node.as_deref(), Some("operator-node"));
    assert_eq!(result.peer.last_seen, None);
    assert_eq!(result.peer.last_error, Some(probe_error));
    assert_eq!(result.peer.nickname.as_deref(), Some("bobby"));
    assert_eq!(result.peer.pubkey.as_deref(), Some("auth-pubkey"));
    assert_eq!(
        result.peer.pubkey_first_seen.as_deref(),
        Some("2026-05-18T12:00:00.000Z")
    );
    assert_eq!(result.peer.one_way, Some(false));
    assert_eq!(
        result.peer.last_symmetric_check.as_deref(),
        Some("2026-05-18T12:00:00.000Z")
    );
    assert_eq!(
        result
            .peer
            .identity
            .as_ref()
            .map(|identity| identity.node.as_str()),
        Some("bob-node")
    );
    assert_eq!(result.peers_after["bob"], result.peer);

    let mut cached = peer("http://old-carol");
    cached.node = Some("old-node".to_owned());
    cached.pubkey = Some("cached-key".to_owned());
    cached.pubkey_first_seen = Some("first-seen".to_owned());
    cached.identity = Some(PeerIdentity {
        oracle: "cached-oracle".to_owned(),
        node: "cached-node".to_owned(),
    });
    cached.one_way = Some(true);
    cached.last_symmetric_check = Some("previous-check".to_owned());
    let readd = cmd_peer_add_from_plan(&PeerAddPlan {
        alias: "carol".to_owned(),
        url: "http://new-carol".to_owned(),
        node: None,
        authenticated_pubkey: None,
        authenticated_identity: None,
        mark_symmetric_check: false,
        one_way: None,
        now: "2026-05-18T12:01:00.000Z".to_owned(),
        peers: BTreeMap::from([("carol".to_owned(), cached)]),
        probe: ProbePeerResult {
            node: Some("new-node".to_owned()),
            nickname: None,
            pubkey: Some("cached-key".to_owned()),
            identity: None,
            error: None,
        },
    })
    .unwrap();

    assert!(readd.overwrote);
    assert_eq!(readd.peer.url, "http://new-carol");
    assert_eq!(readd.peer.node.as_deref(), Some("new-node"));
    assert_eq!(
        readd.peer.last_seen.as_deref(),
        Some("2026-05-18T12:01:00.000Z")
    );
    assert_eq!(readd.peer.pubkey.as_deref(), Some("cached-key"));
    assert_eq!(readd.peer.pubkey_first_seen.as_deref(), Some("first-seen"));
    assert_eq!(
        readd
            .peer
            .identity
            .as_ref()
            .map(|identity| identity.oracle.as_str()),
        Some("cached-oracle")
    );
}

#[test]
fn cmd_peer_probe_mismatch_skips_mutation_and_success_refreshes_identity() {
    let mut erin = peer("http://erin");
    erin.node = Some("old-node".to_owned());
    erin.pubkey = Some("cached-key".to_owned());
    erin.last_seen = Some("old-seen".to_owned());
    let mismatch = cmd_peer_probe_from_plan(&PeerProbePlan {
        alias: "erin".to_owned(),
        now: "2026-05-18T12:00:00.000Z".to_owned(),
        peers: BTreeMap::from([("erin".to_owned(), erin.clone())]),
        probe: ok_probe("rotated-node", Some("new-key")),
        remove_before_mutate: false,
    })
    .unwrap();

    assert_eq!(mismatch.alias, "erin");
    assert!(!mismatch.ok);
    assert_eq!(mismatch.node.as_deref(), Some("rotated-node"));
    assert!(mismatch.pubkey_mismatch.is_some());
    assert_eq!(mismatch.peers_after["erin"], erin);

    let mut dave = peer("http://dave");
    dave.node = Some("seed-node".to_owned());
    dave.nickname = Some("seed-nick".to_owned());
    dave.identity = Some(PeerIdentity {
        oracle: "seed".to_owned(),
        node: "seed-node".to_owned(),
    });
    dave.pubkey = Some("stable-key".to_owned());
    let refreshed = cmd_peer_probe_from_plan(&PeerProbePlan {
        alias: "dave".to_owned(),
        now: "2026-05-18T12:03:00.000Z".to_owned(),
        peers: BTreeMap::from([("dave".to_owned(), dave)]),
        probe: ProbePeerResult {
            node: Some("fresh-node".to_owned()),
            nickname: Some("fresh-nick".to_owned()),
            pubkey: Some("stable-key".to_owned()),
            identity: Some(PeerIdentity {
                oracle: "fresh".to_owned(),
                node: "fresh-node".to_owned(),
            }),
            error: None,
        },
        remove_before_mutate: false,
    })
    .unwrap();

    assert!(refreshed.ok);
    let dave_after = &refreshed.peers_after["dave"];
    assert_eq!(
        dave_after.last_seen.as_deref(),
        Some("2026-05-18T12:03:00.000Z")
    );
    assert_eq!(dave_after.last_error, None);
    assert_eq!(dave_after.node.as_deref(), Some("fresh-node"));
    assert_eq!(dave_after.nickname.as_deref(), Some("fresh-nick"));
    assert_eq!(
        dave_after
            .identity
            .as_ref()
            .map(|identity| identity.oracle.as_str()),
        Some("fresh")
    );
}
