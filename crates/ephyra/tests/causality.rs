// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Causality analysis over traces: `Run::branch` and `loops::causality`.

use ephyra::{
    Config,
    Gen,
    Run,
    Source,
    Trace,
    loops,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Write,
    Read,
    Corrupt,
    Crash,
}

const STEPS: usize = 40;

/// Traces of `STEPS` operations; `Corrupt` is rare and `Crash` rarer still,
/// with the rates given.
struct Ops {
    corrupt: f64,
    crash: f64,
}

impl Gen for Ops {
    type Value = Vec<Op>;

    fn generate(&self, g: &mut Source) -> Vec<Op> {
        self.resume(&Vec::new(), 0, g)
    }
}

impl Trace for Ops {
    fn steps(&self, ops: &Vec<Op>) -> usize {
        ops.len()
    }

    fn resume(&self, ops: &Vec<Op>, prefix: usize, g: &mut Source) -> Vec<Op> {
        let mut ops = ops[..prefix.min(ops.len())].to_vec();
        while ops.len() < STEPS {
            ops.push(
                if g.bool_with(self.corrupt) {
                    Op::Corrupt
                } else if g.bool_with(self.crash) {
                    Op::Crash
                } else if g.bool() {
                    Op::Read
                } else {
                    Op::Write
                },
            );
        }
        ops
    }
}

/// A store whose corruption is only noticed by the integrity check at the
/// end: a delayed failure. `Crash` fails at once, at another site.
fn store(ops: Vec<Op>) {
    let mut corrupted = false;
    for op in ops {
        match op {
            Op::Corrupt => corrupted = true,
            Op::Crash => panic!("crashed"),
            Op::Write | Op::Read => {},
        }
    }
    assert!(!corrupted, "the integrity check failed");
}

fn config(seed: u64) -> Config {
    Config::default().seed(seed).tests(5_000).no_regressions()
}

/// Random testing until the first failure.
fn search<G: Gen<Value = Vec<Op>>, P: FnMut(Vec<Op>)>(run: &mut Run<G, P>) {
    while run.wants_more() {
        let Some(input) = run.draw() else { continue };
        if run.test(&input).failed() {
            return;
        }
    }
}

#[test]
fn the_jump_is_at_the_step_that_corrupted_the_store() {
    let mut run = Run::new(&config(7), Ops { corrupt: 0.01, crash: 0.0 }, store);
    search(&mut run);
    let failing = run.failure().expect("random testing finds a corruption").clone();
    let first =
        failing.iter().position(|op| *op == Op::Corrupt).expect("the failure corrupted the store");

    let causality = loops::causality(&mut run, 32, 8).expect("the failure is analysed");
    assert_eq!(causality.steps, STEPS);
    let jump = causality.jump.clone().expect("the rate rises");
    assert_eq!((jump.from, jump.to), (first, first + 1), "{causality:?}");
    assert_eq!(jump.after, 1.0, "keeping the corruption always fails: {causality:?}");
    assert!(jump.before < 0.5, "without it a continuation rarely corrupts: {causality:?}");
    // Every prefix that keeps the corruption fails every time.
    for point in &causality.points {
        if point.prefix > first {
            assert_eq!(point.failures, point.branches, "{point:?}");
        }
    }

    let report = run.finish();
    assert!(!report.passed());
    assert_eq!(report.causality.as_ref(), Some(&causality));
    let text = report.to_string();
    assert!(text.contains(&format!("causality over {STEPS} steps")), "{text}");
    assert!(text.contains(&format!("the rate rose most at step {}", first + 1)), "{text}");
    #[cfg(feature = "json")]
    assert!(report.to_json().contains("\"causality\""));
}

/// Traces that always end in a crash when resumed early: continuations
/// fail, but at another site, so they do not count as the failure.
struct CrashesWhenResumed;

impl Gen for CrashesWhenResumed {
    type Value = Vec<Op>;

    fn generate(&self, _: &mut Source) -> Vec<Op> {
        vec![Op::Write, Op::Corrupt, Op::Read]
    }
}

impl Trace for CrashesWhenResumed {
    fn steps(&self, ops: &Vec<Op>) -> usize {
        ops.len()
    }

    fn resume(&self, ops: &Vec<Op>, prefix: usize, _: &mut Source) -> Vec<Op> {
        if prefix >= ops.len() {
            return ops.clone();
        }
        let mut ops = ops[..prefix].to_vec();
        ops.push(Op::Crash);
        ops
    }
}

#[test]
fn a_continuation_failing_elsewhere_is_not_the_failure() {
    let mut run = Run::new(&config(1), CrashesWhenResumed, store);
    search(&mut run);
    assert!(run.failure().is_some());
    assert_eq!(run.branch(0), Some(false), "a crash is not the integrity failure");
    assert_eq!(run.branch(3), Some(true), "the whole trace fails as before");
    let causality = loops::causality(&mut run, 4, 4).expect("the failure is analysed");
    let jump = causality.jump.expect("the rate rises at the end");
    assert_eq!(
        (jump.from, jump.to, jump.before, jump.after),
        (2, 3, 0.0, 1.0),
        "{:?}",
        causality.points
    );
}

#[test]
fn without_a_failure_there_is_nothing_to_branch() {
    let mut run = Run::new(&config(1).tests(50), Ops { corrupt: 0.0, crash: 0.0 }, store);
    search(&mut run);
    assert_eq!(run.failure(), None);
    assert_eq!(run.branch(0), None);
    assert_eq!(loops::causality(&mut run, 4, 4), None);
    let report = run.finish();
    assert!(report.passed());
    assert_eq!(report.causality, None);
}
