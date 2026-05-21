use maw_calver::extract_base_from_version;

#[test]
fn invalid_base_with_extra_segment_is_rejected() {
    assert_eq!(extract_base_from_version("26.5.21.1"), None);
}
