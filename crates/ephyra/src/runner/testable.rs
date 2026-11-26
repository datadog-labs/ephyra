// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! What a property may return.

use std::fmt::Debug;

/// The result of one property invocation, as the runner sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The property held.
    Pass,
    /// The property returned `false`.
    False,
    /// The property failed with a message.
    Fail(String),
    /// The input was not applicable; try another one.
    Discard(String),
}

/// An explicit result a property can return when `bool` is not expressive
/// enough. See also the [`assume!`](crate::assume) macro.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TestResult {
    /// The property held.
    Pass,
    /// The property failed; the message goes in the report.
    Fail(String),
    /// The input was not applicable; it is discarded and another is tried.
    Discard(String),
}

impl TestResult {
    /// `Fail` with a message.
    pub fn fail(message: impl Into<String>) -> Self {
        TestResult::Fail(message.into())
    }

    /// `Discard` with a reason.
    pub fn discard(reason: impl Into<String>) -> Self {
        TestResult::Discard(reason.into())
    }

    /// `Pass` if `condition` holds, else `Fail` with `message`.
    pub fn check(condition: bool, message: impl Into<String>) -> Self {
        if condition { TestResult::Pass } else { TestResult::Fail(message.into()) }
    }
}

/// Types a property may return: `bool`, `()`, [`TestResult`], and
/// `Result<T, E: Debug>` of one of these, where `Err` fails and `Ok` takes
/// the verdict of its value. A panic inside the property is a failure as
/// well.
///
/// An `Ok` value of another type does not compile. Map one that is no verdict
/// to `()`:
///
/// ```
/// use ephyra::{check, gens, Config};
///
/// let report = check(&Config::default().tests(20), gens::int(0..=9), |x: i32| {
///     x.to_string().parse::<i32>().map(|_| ())
/// });
/// report.assert_passed();
/// ```
///
/// ```compile_fail,E0277
/// # use ephyra::{check, gens, Config};
/// check(&Config::default().tests(20), gens::int(0..=9), |x: i32| x.to_string().parse::<i32>());
/// ```
#[diagnostic::on_unimplemented(
    message = "a property cannot return `{Self}` or a `Result` of it",
    label = "`{Self}` is not a verdict",
    note = "return `bool`, `()`, `TestResult` or a `Result` of one of these",
    note = "to ignore the `Ok` value of a `Result`, map it to `()`: `.map(|_| ())`"
)]
pub trait Testable {
    /// Convert into the runner's view of the result.
    fn into_outcome(self) -> Outcome;
}

impl Testable for bool {
    fn into_outcome(self) -> Outcome {
        if self { Outcome::Pass } else { Outcome::False }
    }
}

impl Testable for () {
    fn into_outcome(self) -> Outcome {
        Outcome::Pass
    }
}

impl<T: Testable, E: Debug> Testable for Result<T, E> {
    fn into_outcome(self) -> Outcome {
        match self {
            Ok(value) => value.into_outcome(),
            Err(e) => Outcome::Fail(format!("{e:?}")),
        }
    }
}

impl Testable for TestResult {
    fn into_outcome(self) -> Outcome {
        match self {
            TestResult::Pass => Outcome::Pass,
            TestResult::Fail(m) => Outcome::Fail(m),
            TestResult::Discard(r) => Outcome::Discard(r),
        }
    }
}


/// Discard the current input unless `condition` holds. The enclosing
/// property must return [`TestResult`].
///
/// ```
/// use ephyra::{assume, gens, check, Config, TestResult};
///
/// let report = check(&Config::default().tests(20), gens::int(0..=100), |x: i32| {
///     assume!(x % 2 == 0);
///     TestResult::check(x % 2 == 0, "x is even")
/// });
/// assert!(report.passed());
/// assert!(report.discarded > 0);
/// ```
#[macro_export]
macro_rules! assume {
    ($condition:expr) => {
        if !$condition {
            return $crate::TestResult::Discard(
                concat!("assume: ", stringify!($condition)).to_string(),
            );
        }
    };
}
