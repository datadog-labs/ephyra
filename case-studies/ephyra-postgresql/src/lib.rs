// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Model-checked statements and SQLancer-style oracles for PostgreSQL, the
//! port of `../ephyra-sqlite`: a generated database ([`setup::Setup`], a
//! model and the physical choices it is loaded with) and statements over
//! it, which PostgreSQL must run exactly as the model says
//! (`tests/cases.rs`).
//!
//! PostgreSQL itself is master built with assertions, run as a throwaway
//! local cluster ([`pg`]).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

pub mod cases;
pub mod expr;
pub mod fk;
pub mod model;
pub mod pg;
pub mod probes;
pub mod semantics;
pub mod setup;
pub mod sqlancer;
pub mod statements;

/// Mutations of a setup or case that fell back to a fresh draw, counted for
/// a fuzz loop's measurements.
pub static SETUP_FALLBACKS: AtomicU64 = AtomicU64::new(0);

/// Mutations of queries that fell back to fresh queries, counted likewise.
pub static QUERY_FALLBACKS: AtomicU64 = AtomicU64::new(0);

/// Whether the PostgreSQL under test has the fix of a known bug (an id of
/// `postgresql-coverage/deploy/known-bugs.json`): `EPHYRA_FIXED_BUGS`, ids
/// separated by commas, read once. The fuzz runner sets it on a build with
/// the fixes applied (`--fixed-bugs`), and the exclusions of those bugs
/// ([`semantics::gist_dates_ok`], [`sqlancer::known_error`]) are lifted, so
/// that the generator also exercises the fixes. It changes what is
/// generated, so a replay must set it as the run did.
pub fn fixed(bug: &str) -> bool {
    static FIXED: OnceLock<Vec<String>> = OnceLock::new();
    FIXED
        .get_or_init(|| std::env::var("EPHYRA_FIXED_BUGS").unwrap_or_default().split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .iter()
        .any(|b| b == bug)
}

fn fell_back(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}
