// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

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
