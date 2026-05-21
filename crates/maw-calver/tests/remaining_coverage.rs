use maw_calver::{
    compare_bases, compute_version, extract_base_from_version, Channel, ComputeArgs, DateParts,
};

#[test]
fn invalid_bases_with_missing_or_bad_segments_are_rejected() {
    assert_eq!(extract_base_from_version("26"), None);
    assert_eq!(extract_base_from_version("26.5"), None);
    assert_eq!(extract_base_from_version("26.5.21.1"), None);
    assert_eq!(extract_base_from_version("26.5.x"), None);
}

#[test]
#[should_panic(expected = "compareBases expects YY.M.D")]
fn compare_bases_panics_for_invalid_left_base() {
    let _ = compare_bases("26.5", "26.5.21");
}

#[test]
#[should_panic(expected = "compareBases expects YY.M.D")]
fn compare_bases_panics_for_invalid_right_base() {
    let _ = compare_bases("26.5.21", "26.x.21");
}

#[test]
fn compute_version_reports_hhmm_overflow_for_extreme_clock_parts() {
    let err = compute_version(
        ComputeArgs {
            stable: false,
            channel: Some(Channel::Beta),
            now: DateParts {
                year: 2026,
                month: 5,
                day: 21,
                hour: u32::MAX,
                minute: u32::MAX,
            },
        },
        &[],
        "26.5.21",
    )
    .expect_err("saturating u32 stamp cannot fit i32");

    assert_eq!(err, "HHMM stamp overflow");
}
