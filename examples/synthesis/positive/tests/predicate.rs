#[ephyra::generator]
fn positive(value: &u8) -> bool {
    *value > 0
}

#[test]
fn predicate_examples() {
    assert!(!positive(&0));
    assert!(positive(&1));
    assert!(positive(&255));
}
