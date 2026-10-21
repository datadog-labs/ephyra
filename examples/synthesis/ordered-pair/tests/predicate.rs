// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

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
