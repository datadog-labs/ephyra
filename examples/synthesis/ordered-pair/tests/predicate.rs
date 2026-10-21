#[ephyra::generator]
fn ordered(pair: &(u8, u8)) -> bool {
    pair.0 < pair.1
}

#[test]
fn predicate_examples() {
    assert!(ordered(&(0, 1)));
    assert!(ordered(&(254, 255)));
    assert!(!ordered(&(1, 1)));
    assert!(!ordered(&(255, 0)));
}
