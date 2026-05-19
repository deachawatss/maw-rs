use maw_policy::{default_active_group, weight_to_tier, DEFAULT_TIER, KNOWN_TIERS};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureRoot {
    constants: ConstantsFixture,
    weight_to_tier: Vec<WeightFixture>,
    default_active_groups: Vec<DefaultActiveGroupFixture>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConstantsFixture {
    known_tiers: Vec<String>,
    default_tier: String,
}

#[derive(Debug, Deserialize)]
struct WeightFixture {
    name: String,
    weight: i32,
    expected: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DefaultActiveGroupFixture {
    name: String,
    key: String,
    migration: String,
    expected_plugins: Vec<String>,
    excluded_plugins: Vec<String>,
}

#[test]
fn plugin_policy_fixtures_match_maw_js_portable_spec() {
    let fixtures: FixtureRoot =
        serde_json::from_str(include_str!("fixtures/plugin-policy.fixtures.json"))
            .expect("valid plugin policy fixture json");

    let known: Vec<&str> = KNOWN_TIERS.iter().map(|tier| tier.as_str()).collect();
    assert_eq!(
        known, fixtures.constants.known_tiers,
        "known tier constants"
    );
    assert_eq!(
        DEFAULT_TIER.as_str(),
        fixtures.constants.default_tier,
        "default tier constant"
    );

    for fixture in fixtures.weight_to_tier {
        assert_eq!(
            weight_to_tier(fixture.weight).as_str(),
            fixture.expected,
            "weightToTier: {}",
            fixture.name
        );
    }

    for fixture in fixtures.default_active_groups {
        let group = default_active_group(&fixture.key).expect("fixture key has policy group");
        let plugins: Vec<&str> = group.plugins.to_vec();
        assert_eq!(
            plugins, fixture.expected_plugins,
            "default-active plugins: {}",
            fixture.name
        );
        assert_eq!(
            group.migration, fixture.migration,
            "migration: {}",
            fixture.name
        );
        for plugin in fixture.expected_plugins {
            assert!(
                (group.includes)(&plugin),
                "{} should include {plugin}",
                fixture.name
            );
        }
        for plugin in fixture.excluded_plugins {
            assert!(
                !(group.includes)(&plugin),
                "{} should exclude {plugin}",
                fixture.name
            );
        }
    }
}
