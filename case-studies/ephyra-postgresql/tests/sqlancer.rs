// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The SQLancer-style metamorphic oracles, each a property over a case and
//! queries on the state it leaves: NoREC, TLP, differential query plans,
//! ORDER BY and LIMIT, set operations, CTE materialization, window
//! functions, prepared statements, views and EXPLAIN. Their
//! formulations and exclusions are documented in
//! `ephyra_postgresql::sqlancer`.

use ephyra_postgresql::{cases::Case, expr::Query, probes::case_queries, sqlancer};

#[ephyra::test(gen = case_queries(), tests = 1000)]
fn norec_counts_agree(case: Case, queries: Vec<Query>) {
    if let Err(e) = sqlancer::norec(&case, &queries) {
        panic!("{e}");
    }
}

#[ephyra::test(gen = case_queries(), tests = 1000)]
fn tlp_partitions_agree(case: Case, queries: Vec<Query>) {
    if let Err(e) = sqlancer::tlp(&case, &queries) {
        panic!("{e}");
    }
}

#[ephyra::test(gen = case_queries(), tests = 1000)]
fn query_plans_agree(case: Case, queries: Vec<Query>) {
    if let Err(e) = sqlancer::plans(&case, &queries) {
        panic!("{e}");
    }
}

#[ephyra::test(gen = case_queries(), tests = 1000)]
fn ordered_and_limited_rows_agree(case: Case, queries: Vec<Query>) {
    if let Err(e) = sqlancer::order(&case, &queries) {
        panic!("{e}");
    }
}

#[ephyra::test(gen = case_queries(), tests = 1000)]
fn set_operations_agree(case: Case, queries: Vec<Query>) {
    if let Err(e) = sqlancer::setops(&case, &queries) {
        panic!("{e}");
    }
}

#[ephyra::test(gen = case_queries(), tests = 1000)]
fn window_functions_agree(case: Case, queries: Vec<Query>) {
    if let Err(e) = sqlancer::windows(&case, &queries) {
        panic!("{e}");
    }
}

#[ephyra::test(gen = case_queries(), tests = 1000)]
fn prepared_plans_agree(case: Case, queries: Vec<Query>) {
    if let Err(e) = sqlancer::generic(&case, &queries) {
        panic!("{e}");
    }
}

#[ephyra::test(gen = case_queries(), tests = 1000)]
fn cte_materialization_agrees(case: Case, queries: Vec<Query>) {
    if let Err(e) = sqlancer::ctes(&case, &queries) {
        panic!("{e}");
    }
}

#[ephyra::test(gen = case_queries(), tests = 1000)]
fn views_and_explain_agree(case: Case, queries: Vec<Query>) {
    if let Err(e) = sqlancer::views(&case, &queries) {
        panic!("{e}");
    }
}
