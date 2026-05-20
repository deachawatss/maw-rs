use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use maw_peer::{
    default_stale_ttl_ms, empty_peer_store, is_peer_stale, load_peer_store, mutate_peer_store,
    parse_stale_ttl_ms, peer_store_path, remove_stale_peers, save_peer_store, stale_age_ms,
    stale_peer_check, stale_peers, PeerRecord, PeerStoreEnv, PeerStoreFile,
};

fn peer(added_at: &str, last_seen: Option<&str>) -> PeerRecord {
    PeerRecord {
        url: "u".to_owned(),
        node: None,
        added_at: added_at.to_owned(),
        last_seen: last_seen.map(str::to_owned),
        last_error: None,
        nickname: None,
        pubkey: None,
        pubkey_first_seen: None,
        identity: None,
        one_way: None,
        last_symmetric_check: None,
    }
}

#[test]
fn stale_ttl_parsing_matches_maw_js_env_contract() {
    assert_eq!(default_stale_ttl_ms(), 7 * 24 * 60 * 60 * 1000);
    assert_eq!(parse_stale_ttl_ms(None), default_stale_ttl_ms());
    assert_eq!(parse_stale_ttl_ms(Some("1234")), 1234);
    assert_eq!(parse_stale_ttl_ms(Some("0")), default_stale_ttl_ms());
    assert_eq!(parse_stale_ttl_ms(Some("-1")), default_stale_ttl_ms());
    assert_eq!(
        parse_stale_ttl_ms(Some("not-a-number")),
        default_stale_ttl_ms()
    );
    assert_eq!(parse_stale_ttl_ms(Some("")), default_stale_ttl_ms());
}

#[test]
fn stale_age_uses_last_seen_then_added_at_and_clamps_future_dates() {
    let now = 1_779_105_600_000; // 2026-05-18T12:00:00.000Z

    assert_eq!(
        stale_age_ms(&peer("2026-05-18T11:59:50.000Z", None), now),
        Some(10_000)
    );
    assert_eq!(
        stale_age_ms(
            &peer("2026-05-18T00:00:00.000Z", Some("2026-05-18T12:00:05.000Z")),
            now,
        ),
        Some(0)
    );
    assert_eq!(stale_age_ms(&peer("not-date", None), now), None);
}

#[test]
fn is_peer_stale_matches_maw_js_threshold_and_invalid_provenance_rules() {
    let now = 1_779_105_600_000;
    let ten_seconds_old = peer("2026-05-18T11:59:50.000Z", None);

    assert!(is_peer_stale(&ten_seconds_old, 9_999, now));
    assert!(!is_peer_stale(&ten_seconds_old, 10_000, now));
    assert!(is_peer_stale(&peer("not-date", None), 10_000, now));
}

#[test]
fn peer_store_path_empty_stale_tmp_save_and_load_round_trip_match_maw_js() {
    let tmp = TestDir::new("maw-rs-peer-store-round-trip");
    let file = tmp.path().join("nested").join("peers.json");
    let env = PeerStoreEnv::with_vars(
        tmp.path(),
        [("PEERS_FILE", file.to_string_lossy().into_owned())],
    );

    assert_eq!(peer_store_path(&env), file);
    assert_eq!(empty_peer_store(), PeerStoreFile::default());
    assert_eq!(load_peer_store(&env), PeerStoreFile::default());

    let mut peers = BTreeMap::new();
    peers.insert(
        "alpha".to_owned(),
        PeerRecord {
            url: "http://alpha.local:3210".to_owned(),
            node: Some("alpha-node".to_owned()),
            added_at: "2026-05-18T00:00:00.000Z".to_owned(),
            last_seen: None,
            last_error: None,
            nickname: None,
            pubkey: None,
            pubkey_first_seen: None,
            identity: None,
            one_way: None,
            last_symmetric_check: None,
        },
    );
    save_peer_store(&env, &PeerStoreFile { version: 1, peers }).unwrap();
    fs::write(format!("{}.tmp", file.display()), "stale partial write").unwrap();

    assert!(PathBuf::from(format!("{}.tmp", file.display())).exists());
    assert_eq!(
        load_peer_store(&env).peers["alpha"].node.as_deref(),
        Some("alpha-node")
    );
    assert!(!PathBuf::from(format!("{}.tmp", file.display())).exists());
    assert!(fs::read_to_string(file).unwrap().contains("alpha-node"));
}

#[test]
fn state_path_is_primary_while_legacy_home_peers_are_migrated_on_mutation() {
    let tmp = TestDir::new("maw-rs-peer-store-migrate");
    let home = tmp.path().join("home");
    let state = tmp.path().join("state");
    let env = PeerStoreEnv::with_vars(
        &home,
        [("MAW_STATE_DIR", state.to_string_lossy().into_owned())],
    );
    let legacy_file = home.join(".maw").join("peers.json");
    fs::create_dir_all(legacy_file.parent().unwrap()).unwrap();
    fs::write(
        &legacy_file,
        r#"{"version":1,"peers":{"legacy":{"url":"http://legacy.local:3456","node":"legacy-node","addedAt":"2026-05-20T00:00:00.000Z","lastSeen":null}}}"#,
    )
    .unwrap();

    assert_eq!(peer_store_path(&env), state.join("peers.json"));
    assert_eq!(
        load_peer_store(&env).peers["legacy"].node.as_deref(),
        Some("legacy-node")
    );

    let migrated = mutate_peer_store(&env, |data| {
        data.peers.insert(
            "state".to_owned(),
            PeerRecord {
                url: "http://state.local:3456".to_owned(),
                node: Some("state-node".to_owned()),
                added_at: "2026-05-20T01:00:00.000Z".to_owned(),
                last_seen: None,
                last_error: None,
                nickname: None,
                pubkey: None,
                pubkey_first_seen: None,
                identity: None,
                one_way: None,
                last_symmetric_check: None,
            },
        );
    })
    .unwrap();

    assert_eq!(
        migrated.peers.keys().cloned().collect::<Vec<_>>(),
        vec!["legacy", "state"]
    );
    assert_eq!(
        load_peer_store(&env).peers["legacy"].node.as_deref(),
        Some("legacy-node")
    );
    let legacy_after: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(legacy_file).unwrap()).unwrap();
    assert!(legacy_after["peers"]["state"].is_null());
}

#[test]
fn invalid_json_and_invalid_shapes_are_moved_aside_while_callers_get_empty_store() {
    let tmp = TestDir::new("maw-rs-peer-store-corrupt");
    let file = tmp.path().join("peers.json");
    let env = PeerStoreEnv::with_vars(
        tmp.path(),
        [("PEERS_FILE", file.to_string_lossy().into_owned())],
    );

    save_peer_store(&env, &PeerStoreFile::default()).unwrap();
    fs::write(&file, "{not-json").unwrap();
    assert_eq!(load_peer_store(&env), PeerStoreFile::default());
    assert!(!file.exists());
    assert_eq!(load_peer_store(&env), PeerStoreFile::default());

    save_peer_store(&env, &PeerStoreFile::default()).unwrap();
    fs::write(&file, r#"{"version":1,"peers":[]}"#).unwrap();
    assert_eq!(load_peer_store(&env), PeerStoreFile::default());
    assert!(!file.exists());
}

#[test]
fn mutate_peer_store_reads_inside_lock_and_tolerates_malformed_existing_contents() {
    let tmp = TestDir::new("maw-rs-peer-store-mutates");
    let file = tmp.path().join("peers.json");
    let env = PeerStoreEnv::with_vars(
        tmp.path(),
        [("PEERS_FILE", file.to_string_lossy().into_owned())],
    );
    let mut peers = BTreeMap::new();
    peers.insert("before".to_owned(), peer("bad", None));
    save_peer_store(&env, &PeerStoreFile { version: 1, peers }).unwrap();

    let first = mutate_peer_store(&env, |data| {
        data.peers.insert(
            "after".to_owned(),
            PeerRecord {
                url: "http://after".to_owned(),
                node: Some("after-node".to_owned()),
                added_at: "2026-05-18T00:00:00.000Z".to_owned(),
                last_seen: Some("2026-05-18T01:00:00.000Z".to_owned()),
                last_error: None,
                nickname: None,
                pubkey: None,
                pubkey_first_seen: None,
                identity: None,
                one_way: None,
                last_symmetric_check: None,
            },
        );
    })
    .unwrap();
    assert_eq!(
        first.peers.keys().cloned().collect::<Vec<_>>(),
        vec!["after", "before"]
    );
    assert_eq!(
        load_peer_store(&env).peers["after"].node.as_deref(),
        Some("after-node")
    );

    fs::write(&file, r#"{"peers":[]}"#).unwrap();
    let recovered = mutate_peer_store(&env, |data| {
        data.peers.insert(
            "recovered".to_owned(),
            PeerRecord {
                url: "http://recovered".to_owned(),
                node: None,
                added_at: "x".to_owned(),
                last_seen: None,
                last_error: None,
                nickname: None,
                pubkey: None,
                pubkey_first_seen: None,
                identity: None,
                one_way: None,
                last_symmetric_check: None,
            },
        );
    })
    .unwrap();
    assert_eq!(
        recovered.peers.keys().cloned().collect::<Vec<_>>(),
        vec!["recovered"]
    );
    assert_eq!(
        load_peer_store(&env).peers["recovered"].url,
        "http://recovered"
    );
}

#[test]
fn read_errors_and_unlocked_parse_errors_recover_as_empty_stores() {
    let tmp = TestDir::new("maw-rs-peer-store-read-errors");
    let file = tmp.path().join("peers.json");
    let env = PeerStoreEnv::with_vars(
        tmp.path(),
        [("PEERS_FILE", file.to_string_lossy().into_owned())],
    );

    fs::create_dir_all(&file).unwrap();
    assert_eq!(load_peer_store(&env), PeerStoreFile::default());

    fs::remove_dir_all(&file).unwrap();
    fs::write(&file, "{not-json").unwrap();
    let recovered = mutate_peer_store(&env, |data| {
        data.peers.insert(
            "recovered".to_owned(),
            PeerRecord {
                url: "http://recovered".to_owned(),
                node: None,
                added_at: "bad".to_owned(),
                last_seen: None,
                last_error: None,
                nickname: None,
                pubkey: None,
                pubkey_first_seen: None,
                identity: None,
                one_way: None,
                last_symmetric_check: None,
            },
        );
    })
    .unwrap();
    assert_eq!(
        recovered.peers.keys().cloned().collect::<Vec<_>>(),
        vec!["recovered"]
    );
    assert_eq!(
        load_peer_store(&env).peers["recovered"].url,
        "http://recovered"
    );
}

#[test]
fn explicit_stale_cleanup_ignores_missing_and_removes_leftover_tmp_files() {
    let tmp = TestDir::new("maw-rs-peer-store-clear-tmp");
    let file = tmp.path().join("peers.json");
    let env = PeerStoreEnv::with_vars(
        tmp.path(),
        [("PEERS_FILE", file.to_string_lossy().into_owned())],
    );

    maw_peer::clear_stale_peer_store_tmp(&env);
    save_peer_store(&env, &PeerStoreFile::default()).unwrap();
    fs::write(format!("{}.tmp", file.display()), "leftover").unwrap();
    maw_peer::clear_stale_peer_store_tmp(&env);
    assert!(!PathBuf::from(format!("{}.tmp", file.display())).exists());
}

#[test]
fn stale_peer_enumeration_matches_maw_js_stable_ordering_and_age_fallback() {
    let tmp = TestDir::new("maw-rs-stale-peer-enumeration");
    let file = tmp.path().join("peers.json");
    let env = PeerStoreEnv::with_vars(
        tmp.path(),
        [
            ("PEERS_FILE", file.to_string_lossy().into_owned()),
            ("MAW_PEER_STALE_TTL_MS", (7 * DAY_MS).to_string()),
        ],
    );
    save_peer_store(
        &env,
        &store_from([
            (
                "zebra",
                "http://zebra.local",
                iso_days_ago(40),
                Some(iso_days_ago(10)),
            ),
            (
                "fresh",
                "http://fresh.local",
                iso_days_ago(20),
                Some(iso_days_ago(1)),
            ),
            ("alpha", "http://alpha.local", iso_days_ago(8), None),
            (
                "exactTtl",
                "http://exact.local",
                iso_days_ago(30),
                Some(iso_days_ago(7)),
            ),
            (
                "brokenClock",
                "http://broken.local",
                "not-a-date".to_owned(),
                None,
            ),
        ]),
    )
    .unwrap();

    let stale = stale_peers(&env, NOW_MS);
    assert_eq!(stale.len(), 3);
    assert_eq!(stale[0].alias, "alpha");
    assert_eq!(stale[0].url, "http://alpha.local");
    assert_eq!(stale[0].age_ms, Some(8 * DAY_MS));
    assert_eq!(stale[1].alias, "brokenClock");
    assert_eq!(stale[1].url, "http://broken.local");
    assert_eq!(stale[1].age_ms, None);
    assert_eq!(stale[2].alias, "zebra");
    assert_eq!(stale[2].url, "http://zebra.local");
    assert_eq!(stale[2].age_ms, Some(10 * DAY_MS));
}

#[test]
fn stale_peer_check_matches_maw_js_no_stale_singular_and_plural_messages() {
    let tmp = TestDir::new("maw-rs-stale-peer-check");
    let file = tmp.path().join("peers.json");
    let env = PeerStoreEnv::with_vars(
        tmp.path(),
        [
            ("PEERS_FILE", file.to_string_lossy().into_owned()),
            ("MAW_PEER_STALE_TTL_MS", (2 * DAY_MS).to_string()),
        ],
    );
    save_peer_store(
        &env,
        &store_from([("fresh", "http://fresh.local", iso_days_ago(1), None)]),
    )
    .unwrap();
    assert_eq!(stale_peer_check(&env, NOW_MS).name, "peers:stale");
    assert!(stale_peer_check(&env, NOW_MS).ok);
    assert_eq!(stale_peer_check(&env, NOW_MS).message, "no stale peers");

    save_peer_store(
        &env,
        &store_from([("old", "http://old.local", iso_days_ago(3), None)]),
    )
    .unwrap();
    let singular = stale_peer_check(&env, NOW_MS);
    assert!(!singular.ok);
    assert_eq!(
        singular.message,
        "1 stale peer (>2d) — run 'maw doctor --fix-stale' to remove"
    );

    save_peer_store(
        &env,
        &store_from([
            ("old", "http://old.local", iso_days_ago(3), None),
            ("older", "http://older.local", iso_days_ago(4), None),
        ]),
    )
    .unwrap();
    let plural = stale_peer_check(&env, NOW_MS);
    assert!(!plural.ok);
    assert_eq!(
        plural.message,
        "2 stale peers (>2d) — run 'maw doctor --fix-stale' to remove"
    );
}

#[test]
fn remove_stale_peers_preserves_fresh_peers_and_reports_maw_js_messages() {
    let tmp = TestDir::new("maw-rs-stale-peer-remove");
    let file = tmp.path().join("peers.json");
    let env = PeerStoreEnv::with_vars(
        tmp.path(),
        [
            ("PEERS_FILE", file.to_string_lossy().into_owned()),
            ("MAW_PEER_STALE_TTL_MS", (7 * DAY_MS).to_string()),
        ],
    );
    save_peer_store(
        &env,
        &store_from([
            (
                "old",
                "http://old.local",
                iso_days_ago(30),
                Some(iso_days_ago(9)),
            ),
            ("never", "http://never.local", iso_days_ago(10), None),
            (
                "fresh",
                "http://fresh.local",
                iso_days_ago(30),
                Some(iso_days_ago(1)),
            ),
        ]),
    )
    .unwrap();

    let result = remove_stale_peers(&env, NOW_MS).unwrap();
    assert_eq!(result.name, "peers:fix-stale");
    assert!(result.ok);
    assert_eq!(result.message, "removed 2 stale peers");
    assert_eq!(
        load_peer_store(&env)
            .peers
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec!["fresh"]
    );

    let result = remove_stale_peers(&env, NOW_MS).unwrap();
    assert!(result.ok);
    assert_eq!(result.message, "no stale peers");
}

struct TestDir {
    path: PathBuf,
}

const DAY_MS: u64 = 24 * 60 * 60 * 1000;
const NOW_MS: u64 = 1_779_062_400_000; // 2026-05-18T00:00:00.000Z

fn iso_days_ago(days: u64) -> String {
    let ms = NOW_MS - days * DAY_MS;
    match days {
        1 => "2026-05-17T00:00:00.000Z",
        3 => "2026-05-15T00:00:00.000Z",
        4 => "2026-05-14T00:00:00.000Z",
        7 => "2026-05-11T00:00:00.000Z",
        8 => "2026-05-10T00:00:00.000Z",
        9 => "2026-05-09T00:00:00.000Z",
        10 => "2026-05-08T00:00:00.000Z",
        20 => "2026-04-28T00:00:00.000Z",
        30 => "2026-04-18T00:00:00.000Z",
        40 => "2026-04-08T00:00:00.000Z",
        _ => panic!("unexpected fixture day offset {days}; ms={ms}"),
    }
    .to_owned()
}

fn store_from<const N: usize>(peers: [(&str, &str, String, Option<String>); N]) -> PeerStoreFile {
    let peers = peers
        .into_iter()
        .map(|(alias, url, added_at, last_seen)| {
            (
                alias.to_owned(),
                PeerRecord {
                    url: url.to_owned(),
                    node: None,
                    added_at,
                    last_seen,
                    last_error: None,
                    nickname: None,
                    pubkey: None,
                    pubkey_first_seen: None,
                    identity: None,
                    one_way: None,
                    last_symmetric_check: None,
                },
            )
        })
        .collect();
    PeerStoreFile { version: 1, peers }
}

impl TestDir {
    fn new(prefix: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{unique}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
