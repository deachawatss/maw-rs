use maw_matcher::Named;

#[test]
fn borrowed_str_implements_named() {
    let borrowed: &str = "borrowed";
    assert_eq!(borrowed.name(), "borrowed");
}

#[test]
fn owned_string_implements_named_and_no_hint_result_is_none() {
    let owned = String::from("owned");
    assert_eq!(owned.name(), "owned");

    assert_eq!(
        maw_matcher::resolve_by_name("missing", &[owned], maw_matcher::ResolveOptions::default()),
        maw_matcher::ResolveResult::None { hints: None }
    );
}
