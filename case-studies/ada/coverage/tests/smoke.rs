// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! ada's counters are readable in-process, and parsing reaches them.

use ada_coverage::{ada_counters, covered};

fn parse(input: &str) -> impl FnOnce() + '_ {
    move || {
        ephyra_ada::ada::run(ephyra_ada::ada::Kind::Aggregator, &ephyra_ada::Case::parse(input), 0);
    }
}

#[test]
fn parsing_reaches_ada_code() {
    let ada = ada_counters();
    assert!(ada.len() > 1000, "only {} ada counters", ada.len());
    let plain = covered(&ada, parse("https://example.com/a"));
    let ipv4 = covered(&ada, parse("http://0x7f.1/"));
    let ipv6 = covered(&ada, parse("http://[::ffff:1.2.3.4]/"));
    assert!(!plain.is_empty());
    assert!(!ipv4.is_subset(&plain), "IPv4 hosts reach nothing new");
    assert!(!ipv6.is_subset(&plain), "IPv6 hosts reach nothing new");
    println!(
        "ada counters {}, plain {}, ipv4 {}, ipv6 {}",
        ada.len(),
        plain.len(),
        ipv4.len(),
        ipv6.len()
    );
}
