// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! What `#[ephyra::test(search = ...)]` makes of the value its search
//! returns.
#![cfg(feature = "derive")]

use ephyra::{
    Config,
    loops::{
        Fifo,
        fuzz,
    },
};

fn quiet() -> Config {
    Config::default().seed(1).tests(5).no_regressions()
}

fn corpus() -> Result<u64, String> {
    Err("no corpus".to_string())
}

#[ephyra::test(config = quiet(), search = |run| -> Result<(), String> {
    fuzz(run, &mut Fifo::new(), corpus()?);
    Ok(())
})]
#[should_panic(expected = "ephyra: the search returned an error: \"no corpus\"")]
fn an_error_fails_the_test(x: u8) {
    let _ = x;
}

struct NoDebug;

#[ephyra::test(config = quiet(), search = |_run| Err::<(), _>(NoDebug))]
#[should_panic(expected = "ephyra: the search returned an error")]
fn an_error_need_not_be_debug(x: u8) {
    let _ = x;
}

#[ephyra::test(config = quiet(), search = |run| {
    fuzz(run, &mut Fifo::new(), 5);
    Err::<(), _>("stopped")
})]
#[should_panic(expected = "ephyra: FAILED")]
fn fails_in_both_ways(x: u8) -> bool {
    let _ = x;
    false
}

#[test]
fn the_failure_of_the_property_comes_first() {
    let panic = std::panic::catch_unwind(fails_in_both_ways).unwrap_err();
    let report = panic.downcast::<String>().unwrap();
    assert!(report.starts_with("ephyra: FAILED, property returned false"), "{report}");
    assert!(
        report.contains("warning [search_error]: the search returned an error: \"stopped\""),
        "{report}"
    );
}

#[ephyra::test(config = quiet(), search = |run| -> Result<(), NoDebug> {
    fuzz(run, &mut Fifo::new(), 5);
    Ok(())
})]
fn an_ok_passes(x: u8) {
    let _ = x;
}

#[ephyra::test(config = quiet(), search = |run| {
    fuzz(run, &mut Fifo::new(), 5);
    None::<NoDebug>
})]
fn any_other_value_passes(x: u8) {
    let _ = x;
}
