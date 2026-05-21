use maw_calver::{
    compare_bases, compute_version, date_base, effective_base, extract_base_from_version,
    hhmm_stamp, is_valid_calendar_date, max_n_from_package_json, max_n_from_tags,
    next_calendar_base, Channel, ComputeArgs, DateParts,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureRoot {
    date_base: Vec<DateBaseFixture>,
    hhmm_stamp: Vec<DateBaseFixture>,
    extract_base_from_version: Vec<ExtractFixture>,
    compare_bases: Vec<CompareFixture>,
    is_valid_calendar_date: Vec<ValidFixture>,
    next_calendar_base: Vec<NextFixture>,
    max_n_from_tags: Vec<MaxTagsFixture>,
    max_n_from_package_json: Vec<MaxPackageFixture>,
    effective_base: Vec<EffectiveFixture>,
    compute_version: Vec<ComputeFixture>,
}

#[derive(Debug, Deserialize)]
struct DateBaseFixture {
    name: String,
    now: DatePartsFixture,
    expected: String,
}

#[derive(Debug, Deserialize)]
struct DatePartsFixture {
    year: i32,
    month: u32,
    day: u32,
    #[serde(default)]
    hour: u32,
    #[serde(default)]
    minute: u32,
}

impl From<&DatePartsFixture> for DateParts {
    fn from(value: &DatePartsFixture) -> Self {
        Self {
            year: value.year,
            month: value.month,
            day: value.day,
            hour: value.hour,
            minute: value.minute,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ExtractFixture {
    name: String,
    version: String,
    expected: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompareFixture {
    name: String,
    a: String,
    b: String,
    expected_sign: Sign,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum Sign {
    Negative,
    Zero,
    Positive,
}

#[derive(Debug, Deserialize)]
struct ValidFixture {
    name: String,
    base: String,
    expected: bool,
}

#[derive(Debug, Deserialize)]
struct NextFixture {
    name: String,
    base: String,
    expected: String,
}

#[derive(Debug, Deserialize)]
struct MaxTagsFixture {
    name: String,
    base: String,
    channel: ChannelFixture,
    tags: Vec<String>,
    expected: i32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MaxPackageFixture {
    name: String,
    base: String,
    channel: ChannelFixture,
    package_version: String,
    expected: i32,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
enum ChannelFixture {
    #[serde(rename = "alpha")]
    Alpha,
    #[serde(rename = "beta")]
    Beta,
}

impl From<ChannelFixture> for Channel {
    fn from(value: ChannelFixture) -> Self {
        match value {
            ChannelFixture::Alpha => Self::Alpha,
            ChannelFixture::Beta => Self::Beta,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EffectiveFixture {
    name: String,
    today_base: String,
    package_version: String,
    expected: Option<String>,
    error_includes: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ComputeFixture {
    name: String,
    args: ComputeArgsFixture,
    tags: Vec<String>,
    package_version: String,
    expected: String,
    max_suffix: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct ComputeArgsFixture {
    stable: bool,
    channel: Option<ChannelFixture>,
    now: DatePartsFixture,
}

fn sign(value: i32) -> Sign {
    match value.cmp(&0) {
        std::cmp::Ordering::Less => Sign::Negative,
        std::cmp::Ordering::Equal => Sign::Zero,
        std::cmp::Ordering::Greater => Sign::Positive,
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn calver_fixtures_match_maw_js_portable_spec() {
    let fixtures: FixtureRoot = serde_json::from_str(include_str!("fixtures/calver.fixtures.json"))
        .expect("valid calver fixture json");

    for fixture in fixtures.date_base {
        assert_eq!(
            date_base(DateParts::from(&fixture.now)),
            fixture.expected,
            "dateBase: {}",
            fixture.name
        );
    }

    for fixture in fixtures.hhmm_stamp {
        assert_eq!(
            hhmm_stamp(DateParts::from(&fixture.now)),
            fixture.expected,
            "hhmmStamp: {}",
            fixture.name
        );
    }

    for fixture in fixtures.extract_base_from_version {
        assert_eq!(
            extract_base_from_version(&fixture.version),
            fixture.expected,
            "extractBaseFromVersion: {}",
            fixture.name
        );
    }

    for fixture in fixtures.compare_bases {
        assert_eq!(
            sign(compare_bases(&fixture.a, &fixture.b)),
            fixture.expected_sign,
            "compareBases: {}",
            fixture.name
        );
    }

    for fixture in fixtures.is_valid_calendar_date {
        assert_eq!(
            is_valid_calendar_date(&fixture.base),
            fixture.expected,
            "isValidCalendarDate: {}",
            fixture.name
        );
    }

    for fixture in fixtures.next_calendar_base {
        assert_eq!(
            next_calendar_base(&fixture.base),
            fixture.expected,
            "nextCalendarBase: {}",
            fixture.name
        );
    }

    for fixture in fixtures.max_n_from_tags {
        assert_eq!(
            max_n_from_tags(&fixture.base, fixture.channel.into(), &fixture.tags),
            fixture.expected,
            "maxNFromTags: {}",
            fixture.name
        );
    }

    for fixture in fixtures.max_n_from_package_json {
        assert_eq!(
            max_n_from_package_json(
                &fixture.base,
                fixture.channel.into(),
                &fixture.package_version
            ),
            fixture.expected,
            "maxNFromPackageJson: {}",
            fixture.name
        );
    }

    for fixture in fixtures.effective_base {
        let result = effective_base(&fixture.today_base, &fixture.package_version);
        if let Some(error_includes) = fixture.error_includes {
            let err = result.expect_err("effectiveBase fixture should error");
            assert!(
                err.contains(&error_includes),
                "effectiveBase: {} expected error containing {error_includes:?}, got {err:?}",
                fixture.name
            );
        } else {
            assert_eq!(
                result.expect("effectiveBase fixture should succeed"),
                fixture.expected.expect("expected value"),
                "effectiveBase: {}",
                fixture.name
            );
        }
    }

    for fixture in fixtures.compute_version {
        let version = compute_version(
            ComputeArgs {
                stable: fixture.args.stable,
                channel: fixture.args.channel.map(Into::into),
                now: DateParts::from(&fixture.args.now),
            },
            &fixture.tags,
            &fixture.package_version,
        )
        .expect("computeVersion fixture should succeed");
        assert_eq!(
            version, fixture.expected,
            "computeVersion: {}",
            fixture.name
        );
        if let Some(max_suffix) = fixture.max_suffix {
            let suffix = version
                .rsplit('.')
                .next()
                .expect("version suffix")
                .parse::<i32>()
                .expect("numeric suffix");
            assert!(
                suffix <= max_suffix,
                "computeVersion: {} suffix {suffix} > {max_suffix}",
                fixture.name
            );
        }
    }
}

#[test]
fn calver_edge_branches_are_covered() {
    assert_eq!(extract_base_from_version("26..18"), None);
    assert_eq!(extract_base_from_version("26.5.x"), None);
    assert_eq!(extract_base_from_version("26.5.18.1"), None);
    assert!(!is_valid_calendar_date("26.0.1"));
    assert!(!is_valid_calendar_date("26.13.1"));

    let err = effective_base("26.5.18", "26.4.31").unwrap_err();
    assert!(err.contains("day 31 doesn't exist in month 4"));

    assert_eq!(max_n_from_package_json("26.5.18", Channel::Alpha, ""), -1);
    assert_eq!(
        max_n_from_package_json("26.5.18", Channel::Alpha, "26.5.18-alpha.x"),
        -1
    );
}
