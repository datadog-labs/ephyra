// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! What `#[ephyra::test(search = ...)]` does with the value its search
//! returns. Only the macro's expansion uses this module.

use std::fmt::Debug;

use {
    super::Run,
    crate::{
        Gen,
        Report,
    },
};

/// The kind of the health warning that keeps the error in the report.
const KIND: &str = "search_error";

/// The value of a search. `(&&&Returned(value)).__ephyra_search_error()` is
/// `Some` for an `Err`. A search may return any type, so the method comes
/// from the first trait below that fits: the compiler tries the type with
/// the most references first.
pub struct Returned<O>(pub O);

/// A `Result` whose error can be shown.
pub trait ShownError {
    /// The message of an `Err`.
    fn __ephyra_search_error(&self) -> Option<String>;
}

impl<T, E: Debug> ShownError for &&Returned<Result<T, E>> {
    fn __ephyra_search_error(&self) -> Option<String> {
        self.0.as_ref().err().map(|error| format!("the search returned an error: {error:?}"))
    }
}

/// A `Result` whose error cannot be shown.
pub trait HiddenError {
    /// The message of an `Err`.
    fn __ephyra_search_error(&self) -> Option<String>;
}

impl<T, E> HiddenError for &Returned<Result<T, E>> {
    fn __ephyra_search_error(&self) -> Option<String> {
        self.0.is_err().then(|| "the search returned an error".to_string())
    }
}

/// Any other value.
pub trait NoError {
    /// `None`.
    fn __ephyra_search_error(&self) -> Option<String>;
}

impl<O> NoError for Returned<O> {
    fn __ephyra_search_error(&self) -> Option<String> {
        None
    }
}

/// Keep `error` in the report of `run`.
pub fn record<G: Gen, P>(run: &mut Run<G, P>, error: Option<String>) {
    if let Some(error) = error {
        run.report.warn(KIND, error);
    }
}

/// [`Report::assert_passed`], and then panic with the error of the search.
#[track_caller]
pub fn assert_passed(report: &Report) {
    if report.passed()
        && let Some(error) = report.health.iter().find(|warning| warning.kind == KIND)
    {
        panic!("ephyra: {}", error.message);
    }
    report.assert_passed();
}
