use maw_identity::{
    canonical_node_identity, canonical_session_name, canonical_session_stem,
    CanonicalSessionNameInput,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct SessionFixture {
    name: String,
    input: SessionInput,
    expected: String,
}

#[derive(Debug, Deserialize)]
struct SessionInput {
    oracle: String,
    slot: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct NodeFixture {
    name: String,
    input: NodeInput,
    expected: String,
}

#[derive(Debug, Deserialize)]
struct NodeInput {
    host: String,
    user: Option<String>,
}

#[test]
fn canonical_session_name_fixtures_match_maw_js_portable_spec() {
    let fixtures: Vec<SessionFixture> = serde_json::from_str(include_str!(
        "fixtures/canonical-session-name.fixtures.json"
    ))
    .expect("valid canonical session name fixture json");

    for fixture in fixtures {
        let actual = canonical_session_name(&CanonicalSessionNameInput {
            oracle: fixture.input.oracle,
            slot: fixture.input.slot,
        })
        .expect("fixture slot is valid");
        assert_eq!(actual, fixture.expected, "{}", fixture.name);
    }
}

#[test]
fn canonical_node_identity_fixtures_match_maw_js_portable_spec() {
    let fixtures: Vec<NodeFixture> = serde_json::from_str(include_str!(
        "fixtures/canonical-node-identity.fixtures.json"
    ))
    .expect("valid canonical node identity fixture json");

    for fixture in fixtures {
        let actual = canonical_node_identity(&fixture.input.host, fixture.input.user.as_deref());
        assert_eq!(actual, fixture.expected, "{}", fixture.name);
    }
}

#[test]
fn constructors_stem_and_slot_errors_are_covered() {
    assert_eq!(
        canonical_session_name(&CanonicalSessionNameInput::new("50-foo-oracle.git")).unwrap(),
        "foo"
    );
    assert_eq!(
        canonical_session_name(&CanonicalSessionNameInput::with_slot("foo-oracle", 7)).unwrap(),
        "07-foo"
    );
    assert_eq!(canonical_session_stem("foo-oracle").unwrap(), "foo");
    assert_eq!(
        canonical_session_name(&CanonicalSessionNameInput::with_slot("foo", 100)).unwrap_err(),
        "invalid fleet slot '100'"
    );
}
