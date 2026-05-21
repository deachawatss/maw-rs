use maw_matcher::Named;

#[test]
fn borrowed_str_implements_named() {
    let borrowed: &str = "borrowed";
    assert_eq!(borrowed.name(), "borrowed");
}
