#[ephyra::generator]
fn sorted(values: &[u8]) -> bool {
    values.windows(2).all(|pair| pair[0] <= pair[1])
}

#[test]
fn predicate_examples() {
    assert!(sorted(&[]));
    assert!(sorted(&[0, 0, 255]));
    assert!(!sorted(&[2, 1]));
}
