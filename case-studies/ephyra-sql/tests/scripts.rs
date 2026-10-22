// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

use ephyra::{Config, Gen, Run, Source, Trace, loops};
use ephyra_sql::engine::{Outcome, Sqlite, Value};
use ephyra_sql::script::{self, Script, ScriptGen, scripts};

/// Turso and SQLite agree after every step of a script. On a failure the
/// search runs causality analysis before shrinking.
#[ephyra::test(gen = scripts(), config = ephyra_sql::config(), search = ephyra_sql::explore_and_explain)]
fn scripts_agree(script: Script) {
    ephyra::feature("steps", script.steps.len());
    ephyra::feature("steps failing in both", script::differential(&script));
}

#[test]
fn resuming_keeps_the_prefix_and_the_length() {
    let mut g = Source::new(5, 40);
    let script = ScriptGen.generate(&mut g);
    let n = script.steps.len();
    assert_eq!(ScriptGen.resume(&script, n, &mut g), script, "resuming at the end changes nothing");
    let resumed = ScriptGen.resume(&script, n / 2, &mut g);
    assert_eq!(resumed.steps[..n / 2], script.steps[..n / 2]);
    assert_eq!(resumed.steps.len(), n);
}

const LIMIT: i64 = 60;

/// Rows in the table after each step of `script`, in SQLite.
fn sizes(script: &Script) -> Vec<i64> {
    let db = Sqlite::open();
    db.run(&ephyra_sql::ast::create(&script.schema));
    let arity = script.schema.len();
    script
        .steps
        .iter()
        .map(|step| {
            db.run(&step.sql(arity));
            match db.run("SELECT count(*) FROM t") {
                Outcome::Rows(rows) => match rows[0][0] {
                    Value::Integer(n) => n,
                    _ => 0,
                },
                Outcome::Error(e) => panic!("{e}"),
            }
        })
        .collect()
}

/// A planted delayed failure: the check at the end fails when the table
/// ever held more than `LIMIT` rows. Causality analysis must place the rise
/// no later than the step that first went over.
#[test]
fn causality_finds_the_step_that_went_over() {
    let config = Config::default().seed(11).tests(2_000).no_regressions();
    let mut run = Run::new(&config, ScriptGen, |script: Script| {
        let over = sizes(&script).iter().any(|&n| n > LIMIT);
        assert!(!over, "the table grew past {LIMIT} rows");
    });
    while run.wants_more() {
        let Some(input) = run.draw() else { continue };
        if run.test(&input).failed() {
            break;
        }
    }
    let failing = run.failure().expect("some script grows past the limit").clone();
    let first = sizes(&failing).iter().position(|&n| n > LIMIT).expect("it went over");

    let causality = loops::causality(&mut run, 16, 8).expect("the failure is analysed");
    for point in &causality.points {
        if point.prefix > first {
            assert_eq!(
                point.failures,
                point.branches,
                "keeping step {} always fails: {point:?}",
                first + 1
            );
        }
    }
    let jump = causality.jump.as_ref().expect("the rate rises");
    assert!(jump.to <= first + 1, "the rise ends by step {}: {causality:?}", first + 1);
    let report = run.finish();
    assert!(report.to_string().contains("causality over"), "{report}");
}
