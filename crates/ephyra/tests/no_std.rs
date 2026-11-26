// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! `#[ephyra::test]` compiles in a crate that does not link `std` itself.
#![no_std]
#![cfg(feature = "derive")]

#[ephyra::test(tests = 10)]
fn the_expansion_does_not_name_std(x: u8) -> bool {
    x.count_ones() <= 8
}
