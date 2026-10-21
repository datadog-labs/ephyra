//! Labels, classification and target scores: the property's side channel
//! to the runner.
//!
//! Call [`label`] or [`classify`] from inside a property to see how inputs
//! are distributed, [`feature`] to record a number or a category per input,
//! [`observe`] and [`measure`] to add values only to an enabled observation
//! trace, [`cover`] to require a share of inputs to meet a condition, and
//! [`target`] to steer the search toward inputs that maximise a score. Outside
//! a running property all of them are no-ops.

use std::cell::RefCell;

/// A value recorded with [`feature`]: a number, summarised in the report
/// with its quantiles and a histogram, or a piece of text, counted like a
/// label.
#[derive(Clone, Debug, PartialEq)]
pub enum FeatureValue {
    /// A number; non-finite values are ignored.
    Number(f64),
    /// A category.
    Text(String),
}

macro_rules! numeric_features {
    ($($t:ty),*) => {$(
        impl From<$t> for FeatureValue {
            fn from(value: $t) -> Self {
                FeatureValue::Number(value as f64)
            }
        }
    )*};
}
numeric_features!(f64, f32, i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize);

macro_rules! text_features {
    ($($t:ty),*) => {$(
        impl From<$t> for FeatureValue {
            fn from(value: $t) -> Self {
                FeatureValue::Text(value.to_string())
            }
        }
    )*};
}
text_features!(bool, char, &str);

impl From<String> for FeatureValue {
    fn from(value: String) -> Self {
        FeatureValue::Text(value)
    }
}

#[derive(Default)]
struct Context {
    labels: Vec<String>,
    scores: Vec<f64>,
    covers: Vec<(String, bool)>,
    features: Vec<(String, FeatureValue)>,
    /// Trace-only observations. `None` keeps their lazy values unevaluated.
    observations: Option<Vec<(String, FeatureValue)>>,
}

thread_local! {
    static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) };
}

/// Apply `f` to the current test case's context, when a property is running.
fn record(f: impl FnOnce(&mut Context)) {
    CONTEXT.with(|context| {
        if let Some(context) = context.borrow_mut().as_mut() {
            f(context);
        }
    });
}

/// Tag the current test case with `name`. The report shows what share of
/// passing tests carried each label.
pub fn label(name: impl Into<String>) {
    record(|context| context.labels.push(name.into()));
}

/// Tag the current test case with `name` when `condition` holds.
pub fn classify(condition: bool, name: impl Into<String>) {
    if condition {
        label(name);
    }
}

/// Record a feature of the current input: a number (a length, a depth, a
/// sum) or a category (a variant name, a bucket). The report summarises
/// each numeric feature over the passing tests with its minimum, median,
/// 90th percentile, maximum and a histogram, and counts each text value
/// like a [`label`] named `feature=value`. Use it to see the shape of what
/// the generator produces without printing inputs:
///
/// ```
/// use ephyra::{check, feature, gens, Config};
///
/// let report = check(&Config::default().tests(200), gens::vec(gens::int(0i32..=9)), |v| {
///     feature("len", v.len());
///     feature("parity", if v.len().is_multiple_of(2) { "even" } else { "odd" });
///     true
/// });
/// assert!(report.features["len"].max <= 100.0);
/// assert!(report.labels.contains_key("parity=even"));
/// ```
pub fn feature(name: impl Into<String>, value: impl Into<FeatureValue>) {
    let value = value.into();
    if let FeatureValue::Number(n) = value
        && !n.is_finite()
    {
        return;
    }
    record(|context| context.features.push((name.into(), value)));
}

/// Lazily record a value on the current test case's observation line.
///
/// Unlike [`feature`], an observation is not aggregated into the final
/// [`Report`](crate::Report). It is evaluated and recorded only while a sink
/// is open for the run (the observation file of
/// [`Config::report.observations`](crate::config::ReportConfig::observations),
/// or one added with [`sink::install`](crate::sink::install)), so structural
/// measurements and other profiling probes add no work to ordinary test runs.
///
/// Numeric observations can be plotted over the run by the Ephyra ETNA
/// profile renderer. Text observations remain available to other trace
/// consumers.
///
/// ```
/// use ephyra::{Config, check, gens, observe};
///
/// let config = Config::default().tests(10);
/// let report = check(&config, gens::vec(gens::int(0..=9)), |values| {
///     observe("input elements", || values.len());
///     true
/// });
/// assert!(report.passed());
/// ```
pub fn observe<T>(name: impl Into<String>, value: impl FnOnce() -> T)
where
    T: Into<FeatureValue>,
{
    CONTEXT.with(|context| {
        let mut context = context.borrow_mut();
        let Some(observations) = context.as_mut().and_then(|c| c.observations.as_mut()) else {
            return;
        };
        let value = value().into();
        if matches!(value, FeatureValue::Number(n) if !n.is_finite()) {
            return;
        }
        observations.push((name.into(), value));
    });
}

/// Time `operation` and record its duration as a trace-only observation.
///
/// The operation always runs. The timer and observation bookkeeping are used
/// only while a sink is open (see [`observe`]). The recorded name has ` (µs)`
/// appended so profile plots carry their unit.
pub fn measure<R>(name: impl Into<String>, operation: impl FnOnce() -> R) -> R {
    let recording =
        CONTEXT.with(|context| context.borrow().as_ref().is_some_and(|c| c.observations.is_some()));
    if !recording {
        return operation();
    }
    let started = std::time::Instant::now();
    let result = operation();
    let micros = started.elapsed().as_secs_f64() * 1_000_000.0;
    observe(format!("{} (µs)", name.into()), || micros);
    result
}

/// Report a score for the current input. When a property calls this, the
/// runner follows the random phase with a targeted phase that mutates the
/// best-scoring inputs to push the score higher (see
/// [`Config::target`](crate::Config::target)); use it to steer toward
/// slow paths, deep recursion, large sums, or any measure that correlates
/// with the bug you are hunting. The highest score and the input that
/// reached it are in the report. Several calls in one test case count as
/// their maximum. Non-finite scores are ignored.
pub fn target(score: f64) {
    if !score.is_finite() {
        return;
    }
    record(|context| context.scores.push(score));
}

/// Declare a case the inputs must exercise: like [`classify`], but the
/// runner reports the share of passing tests that met `condition` under
/// `label`, and warns when it falls below
/// [`Config::coverage.min_cover`](crate::config::CoverageConfig::min_cover).
pub fn cover(condition: bool, label: impl Into<String>) {
    record(|context| context.covers.push((label.into(), condition)));
}

/// What a property reported through the side channel.
pub(crate) struct Observed {
    pub labels: Vec<String>,
    /// The maximum of the scores passed to [`target`], if any.
    pub score: Option<f64>,
    /// Cover labels and whether their condition held.
    pub covers: Vec<(String, bool)>,
    /// Features recorded with [`feature`], in call order.
    pub features: Vec<(String, FeatureValue)>,
    /// Trace-only values recorded with [`observe`], in call order.
    pub observations: Vec<(String, FeatureValue)>,
}

/// Run `f` with the side channel active and return what it reported. Nested
/// calls report to their own caller: a property run inside a check does not
/// reach the check's labels and features.
pub(crate) fn collect<R>(trace_observations: bool, f: impl FnOnce() -> R) -> (R, Observed) {
    let current = Context { observations: trace_observations.then(Vec::new), ..Context::default() };
    let outer = CONTEXT.with(|context| context.borrow_mut().replace(current));
    let result = f();
    let context = CONTEXT
        .with(|context| std::mem::replace(&mut *context.borrow_mut(), outer))
        .unwrap_or_default();
    let score = context
        .scores
        .iter()
        .copied()
        .fold(None, |best: Option<f64>, s| Some(best.map_or(s, |b| b.max(s))));
    (
        result,
        Observed {
            labels: context.labels,
            score,
            covers: context.covers,
            features: context.features,
            observations: context.observations.unwrap_or_default(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_channel_is_active_only_inside_collect() {
        label("ignored");
        target(1.0);
        let ((), observed) = collect(true, || {
            label("a");
            classify(true, "b");
            classify(false, "c");
            target(2.0);
            target(f64::NAN);
            target(5.0);
            target(3.0);
            cover(true, "c");
            cover(false, "d");
            feature("len", 3usize);
            feature("kind", "leaf");
            feature("nan", f64::NAN);
            observe("nodes", || 11usize);
            observe("phase", || "normalize");
            observe("nan observation", || f64::NAN);
            assert_eq!(measure("work", || 42), 42);
        });
        assert_eq!(observed.labels, vec!["a", "b"]);
        assert_eq!(observed.score, Some(5.0));
        assert_eq!(observed.covers, vec![("c".to_string(), true), ("d".to_string(), false)]);
        assert_eq!(
            observed.features,
            vec![
                ("len".to_string(), FeatureValue::Number(3.0)),
                ("kind".to_string(), FeatureValue::Text("leaf".to_string()))
            ]
        );
        assert_eq!(observed.observations[0], ("nodes".to_string(), FeatureValue::Number(11.0)));
        assert_eq!(
            observed.observations[1],
            ("phase".to_string(), FeatureValue::Text("normalize".to_string()))
        );
        assert_eq!(observed.observations[2].0, "work (µs)");
        assert!(matches!(&observed.observations[2].1, FeatureValue::Number(n) if n.is_finite()));
        let called = std::cell::Cell::new(false);
        let ((), none) = collect(false, || {
            observe("lazy", || {
                called.set(true);
                1usize
            });
            assert_eq!(measure("work", || 7), 7);
        });
        assert!(!called.get());
        assert!(none.labels.is_empty());
        assert_eq!(none.score, None);
        assert!(none.observations.is_empty());
    }
}
