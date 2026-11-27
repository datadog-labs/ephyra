// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! A panic in the code that runs after a discard counts as that discard.

use ephyra::{
    BoxGen,
    Config,
    Gen,
    GenExt,
    Run,
    Source,
    Status,
    check,
    gens,
};

fn cfg() -> Config {
    Config::default().seed(1).tests(200).no_regressions()
}

/// Always discards the first input, whose size is 0.
fn non_empty() -> impl Gen<Value = Vec<u8>> {
    gens::vec(gens::int(0u8..=9)).filter("non-empty", |v: &Vec<u8>| !v.is_empty())
}

#[test]
fn partial_code_after_a_filter_is_discarded_with_the_filters_reason() {
    let generators: [(&str, BoxGen<u8>); 5] = [
        ("map", non_empty().map(|v| v[0]).boxed()),
        ("dependent", non_empty().dependent(|v| gens::int(0..v.len())).map(|(v, i)| v[i]).boxed()),
        ("and_then", non_empty().and_then(gens::pick_from).boxed()),
        ("filter", non_empty().filter("head", |v| v[0] <= 9).map(|v| v[0]).boxed()),
        ("draw", BoxGen::new(|g: &mut Source| g.draw(&non_empty())[0])),
    ];
    for (name, generator) in generators {
        let report = check(&cfg(), generator, |x| x <= 9);
        assert!(report.passed(), "{name}: {report}");
        assert_eq!(report.discarded, 1, "{name}: {report}");
        assert_eq!(report.discard_reasons["filter:non-empty"], 1, "{name}: {report}");
    }
}

#[test]
fn a_discard_does_not_hide_a_failure() {
    let pairs = non_empty().map(|v| (v[0], v));
    let report = check(&cfg(), pairs, |(_, v): (u8, Vec<u8>)| v.len() < 5);
    assert_eq!(report.status, Status::Failed, "{report}");
}

#[test]
fn a_panic_without_a_discard_still_aborts() {
    let report = check(&cfg(), gens::vec(gens::int(0u8..=9)).map(|v| v[0]), |x: u8| x <= 9);
    assert_eq!(report.status, Status::Aborted, "{report}");
    assert!(report.abort.unwrap().message.contains("index out of bounds"));
}

#[test]
fn mutation_counts_the_discard_too() {
    let generator = gens::constant(2u8).with_mutate(|_, g| {
        g.discard("no mutant");
        panic!("after the discard")
    });
    let mut run = Run::new(&cfg(), generator, |_| true);
    assert!(run.mutate(&2).is_none());
    let report = run.finish();
    assert!(report.abort.is_none(), "{report}");
    assert_eq!(report.discard_reasons["no mutant"], 1);
}
