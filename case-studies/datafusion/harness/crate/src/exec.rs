// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Runs one case under the baseline and each variant configuration and judges it.

use std::sync::{Arc, Mutex};

use datafusion::arrow::array::{ArrayRef, BooleanArray, Float64Array, Int32Array, Int64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::arrow::util::display::{ArrayFormatter, FormatOptions};
use datafusion::datasource::MemTable;
use datafusion::prelude::*;

use crate::case::{Case, Table, Ty, Variant, Verdict};

/// Panics observed by the hook since the last `take_panics()` (message @ file:line).
pub static PANICS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic>".into()
        };
        let loc = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        if let Ok(mut p) = PANICS.lock() {
            p.push((msg, loc));
        }
    }));
}

fn take_panics() -> Vec<(String, String)> {
    std::mem::take(&mut *PANICS.lock().unwrap())
}

pub enum Outcome {
    Rows(Vec<String>),
    Err(String),
    Panic(String, String),
}

fn arrow_type(t: Ty) -> DataType {
    match t {
        Ty::I64 => DataType::Int64,
        Ty::I32 => DataType::Int32,
        Ty::F64 => DataType::Float64,
        Ty::Str => DataType::Utf8,
        Ty::Bool => DataType::Boolean,
    }
}

fn column(t: Ty, vals: &[&Option<String>]) -> ArrayRef {
    match t {
        Ty::I64 => Arc::new(Int64Array::from_iter(vals.iter().map(|v| v.as_ref().map(|s| s.parse::<i64>().unwrap())))),
        Ty::I32 => Arc::new(Int32Array::from_iter(vals.iter().map(|v| v.as_ref().map(|s| s.parse::<i32>().unwrap())))),
        Ty::F64 => Arc::new(Float64Array::from_iter(vals.iter().map(|v| v.as_ref().map(|s| s.parse::<f64>().unwrap())))),
        Ty::Str => Arc::new(StringArray::from_iter(vals.iter().map(|v| v.as_deref()))),
        Ty::Bool => Arc::new(BooleanArray::from_iter(vals.iter().map(|v| v.as_ref().map(|s| s == "true")))),
    }
}

fn mem_table(t: &Table) -> datafusion::error::Result<MemTable> {
    let schema = Arc::new(Schema::new(
        t.cols.iter().map(|c| Field::new(&c.name, arrow_type(c.ty), true)).collect::<Vec<_>>(),
    ));
    let mut parts = Vec::new();
    let mut at = 0;
    for &n in &t.parts {
        let mut batches = Vec::new();
        let end = at + n;
        while at < end {
            let len = t.batch.max(1).min(end - at);
            let rows = &t.rows[at..at + len];
            let cols: Vec<ArrayRef> = t
                .cols
                .iter()
                .enumerate()
                .map(|(i, c)| column(c.ty, &rows.iter().map(|r| &r[i]).collect::<Vec<_>>()))
                .collect();
            batches.push(RecordBatch::try_new(schema.clone(), cols)?);
            at += len;
        }
        parts.push(batches);
    }
    if parts.is_empty() {
        parts.push(vec![]);
    }
    MemTable::try_new(schema, parts)
}

async fn session(case: &Case, v: &Variant) -> Result<SessionContext, String> {
    let mut cfg = SessionConfig::new().with_target_partitions(4);
    for (k, val) in &v.settings {
        cfg.options_mut().set(k, val).map_err(|e| format!("HARNESS bad setting {k}={val}: {e}"))?;
    }
    let ctx = SessionContext::new_with_config(cfg);
    for r in &v.remove_rules {
        if !ctx.remove_optimizer_rule(r) {
            return Err(format!("HARNESS unknown optimizer rule {r}"));
        }
    }
    for t in &case.tables {
        let mt = mem_table(t).map_err(|e| format!("HARNESS table: {e}"))?;
        ctx.register_table(t.name.as_str(), Arc::new(mt)).map_err(|e| format!("HARNESS register: {e}"))?;
    }
    Ok(ctx)
}

/// The optimized logical plan of a configuration, as indented text (subqueries included).
async fn optimized_plan(case: &Case, v: &Variant) -> Option<String> {
    let ctx = session(case, v).await.ok()?;
    let df = ctx.sql(&case.query).await.ok()?;
    df.into_optimized_plan().ok().map(|p| format!("{}", p.display_indent()))
}

/// Optimized logical plans of the base and every variant (debugging aid: `dfz plan case.json`).
pub fn plans(rt: &tokio::runtime::Runtime, case: &Case) -> Vec<(String, Option<String>)> {
    std::iter::once(&case.base)
        .chain(case.variants.iter())
        .map(|v| (v.name.clone(), rt.block_on(optimized_plan(case, v))))
        .collect()
}

/// True when the variant's optimizer proved part of the query empty and dropped inputs the
/// base plan still reads: the variant has fewer table scans, and either an `EmptyRelation: rows=0`
/// or fewer outer references (an empty UNION arm holding the correlated subquery is removed
/// outright, leaving no EmptyRelation behind). Decorrelating a subquery keeps its scans (it
/// becomes a join), so this does not match a variant that succeeded by decorrelating what the
/// base left behind.
async fn variant_folded_away_inputs(case: &Case, base: &Variant, v: &Variant) -> bool {
    let (Some(b), Some(p)) = (optimized_plan(case, base).await, optimized_plan(case, v).await) else {
        return false;
    };
    let fewer = |pat: &str| p.matches(pat).count() < b.matches(pat).count();
    fewer("TableScan") && (p.contains("EmptyRelation: rows=0") || fewer("outer_ref("))
}

/// 0.0 and -0.0 compare equal, so DISTINCT / GROUP BY / UNION keep either one as the group's value and
/// which one depends on input order (plan shape). Render -0.0 as 0.0 so that choice is not a difference.
fn positive_zero(c: &ArrayRef) -> ArrayRef {
    match c.as_any().downcast_ref::<Float64Array>() {
        Some(a) if a.iter().flatten().any(|v| v == 0.0 && v.is_sign_negative()) => {
            Arc::new(a.iter().map(|v| v.map(|v| if v == 0.0 { 0.0 } else { v })).collect::<Float64Array>())
        }
        _ => c.clone(),
    }
}

async fn run_query(case: &Case, v: &Variant) -> Result<Vec<String>, String> {
    let ctx = session(case, v).await?;
    let df = ctx.sql(&case.query).await.map_err(|e| e.to_string())?;
    let batches = df.collect().await.map_err(|e| e.to_string())?;
    let opts = FormatOptions::default().with_null("NULL");
    let mut rows = Vec::new();
    for b in &batches {
        let cols: Vec<ArrayRef> = b.columns().iter().map(positive_zero).collect();
        let fmts: Vec<ArrayFormatter> = cols
            .iter()
            .map(|c| ArrayFormatter::try_new(c.as_ref(), &opts))
            .collect::<Result<_, _>>()
            .map_err(|e| format!("HARNESS format: {e}"))?;
        for i in 0..b.num_rows() {
            let cells: Vec<String> = fmts.iter().map(|f| f.value(i).to_string()).collect();
            rows.push(cells.join(" | "));
        }
    }
    Ok(rows)
}

pub fn run_variant(rt: &tokio::runtime::Runtime, case: &Case, v: &Variant) -> Outcome {
    take_panics();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rt.block_on(run_query(case, v))));
    let panics = take_panics();
    if let Some((m, l)) = panics.into_iter().next() {
        return Outcome::Panic(m, l);
    }
    match r {
        Ok(Ok(rows)) => Outcome::Rows(rows),
        Ok(Err(e)) => Outcome::Err(e),
        Err(_) => Outcome::Panic("<panic without hook>".into(), String::new()),
    }
}

fn file_stem(loc: &str) -> String {
    let file = loc.rsplit_once(':').map(|(f, _)| f).unwrap_or(loc);
    let base = file.rsplit('/').next().unwrap_or(file);
    let mut stem = base.strip_suffix(".rs").unwrap_or(base).to_string();
    if stem == "mod" {
        // name a module by its directory (topk/mod.rs -> topk_mod)
        let parent = file.rsplit('/').nth(1).unwrap_or("");
        stem = format!("{parent}_mod");
    }
    let s: String = stem
        .chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    if s.is_empty() { "unknown".into() } else { s }
}

/// Errors that are about the harness or the environment, never a target verdict.
fn is_resource(e: &str) -> bool {
    e.contains("Resources exhausted") || e.contains("HARNESS")
}

fn crash_sig(e: &str) -> Option<String> {
    if e.contains("Internal error") {
        Some(format!("crash__internal_error__{}", internal_error_area(e)))
    } else if e.contains("Optimizer rule '") && e.contains("failed") {
        // different rules failing are different bugs
        Some(format!("crash__optimizer_rule_failed__{}", optimizer_rule_name(e)))
    } else {
        None
    }
}

/// The rule in "Optimizer rule 'push_down_limit' failed ..." (push_down_limit).
fn optimizer_rule_name(e: &str) -> String {
    let rule = e
        .split_once("Optimizer rule '")
        .and_then(|(_, r)| r.split_once('\''))
        .map(|(r, _)| r)
        .unwrap_or("");
    if rule.is_empty() || rule.len() > 60 || !rule.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        "unknown".into()
    } else {
        rule.to_ascii_lowercase()
    }
}

/// Names an internal error by the leading plain words of its message
/// ("Internal error: Physical input schema should be ..." -> physical_input_schema_should),
/// stopping at the first token with digits, quotes or punctuation so no values get in.
fn internal_error_area(e: &str) -> String {
    let rest = e.split_once("Internal error: ").map(|(_, r)| r).unwrap_or("");
    let words: Vec<String> = rest
        .split_whitespace()
        .take_while(|w| w.chars().all(|c| c.is_ascii_alphabetic()))
        .take(4)
        .map(|w| w.to_ascii_lowercase())
        .collect();
    if words.is_empty() { "unknown".into() } else { words.join("_") }
}

fn short(s: &str) -> String {
    let s = s.replace('\n', " ");
    if s.len() > 600 { format!("{}...", &s[..s.char_indices().nth(600).map(|(i, _)| i).unwrap_or(s.len())]) } else { s }
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

fn diff_msg(a: &[String], b: &[String]) -> String {
    let (sa, sb) = (sorted(a.to_vec()), sorted(b.to_vec()));
    let only_a: Vec<&String> = sa.iter().filter(|r| !sb.contains(r)).take(5).collect();
    let only_b: Vec<&String> = sb.iter().filter(|r| !sa.contains(r)).take(5).collect();
    format!(
        "base {} rows, variant {} rows; only in base: {:?}; only in variant: {:?}",
        a.len(),
        b.len(),
        only_a,
        only_b
    )
}

pub fn judge(rt: &tokio::runtime::Runtime, case: &Case) -> Verdict {
    let mut verdict = Verdict::default();
    let base = run_variant(rt, case, &case.base);
    match &base {
        Outcome::Panic(m, l) => {
            verdict.sig = Some(format!("crash__panic_{}", file_stem(l)));
            verdict.msg = format!("[{}] panic: {} at {}", case.base.name, short(m), l);
            return verdict;
        }
        Outcome::Err(e) => {
            if let Some(s) = crash_sig(e) {
                verdict.sig = Some(s);
                verdict.msg = format!("[{}] {}", case.base.name, short(e));
                return verdict;
            }
            verdict.base_err = Some(short(e));
        }
        Outcome::Rows(_) => {}
    }
    for v in &case.variants {
        let out = run_variant(rt, case, v);
        // Removing an optimizer rule, or capping the optimizer to one pass, may leave a
        // plan the physical planner was never meant to see (un-decorrelated subqueries,
        // un-simplified coalesce). Errors there are not correctness claims.
        let rule_variant = !v.remove_rules.is_empty() || v.name == "max_passes1";
        match (&base, &out) {
            (_, Outcome::Panic(m, l)) => {
                verdict.sig = Some(format!("crash__panic_{}", file_stem(l)));
                verdict.msg = format!("[{}] panic: {} at {}", v.name, short(m), l);
                return verdict;
            }
            (_, Outcome::Err(e)) if crash_sig(e).is_some() && rule_variant => {
                verdict.notes.push("rule_removed_internal".into());
            }
            (_, Outcome::Err(e)) if crash_sig(e).is_some() => {
                verdict.sig = crash_sig(e);
                verdict.msg = format!("[{}] {}", v.name, short(e));
                return verdict;
            }
            (Outcome::Rows(a), Outcome::Rows(b)) => {
                let same = if case.ordered { a == b } else { sorted(a.clone()) == sorted(b.clone()) };
                if !same {
                    verdict.sig = Some(format!("toggle__rows_{}", v.name));
                    verdict.msg = format!("[{}] {}", v.name, diff_msg(a, b));
                    return verdict;
                }
            }
            (Outcome::Err(_), Outcome::Err(_)) => verdict.notes.push("both_error".into()),
            (Outcome::Rows(_), Outcome::Err(e)) | (Outcome::Err(e), Outcome::Rows(_)) => {
                let base_failed = matches!(base, Outcome::Err(_));
                if is_resource(e) {
                    verdict.notes.push("resource_error".into());
                } else if base_failed
                    && e.contains("Physical plan does not support logical expression")
                    && rt.block_on(variant_folded_away_inputs(case, &case.base, v))
                {
                    // Subqueries the decorrelation rules cannot rewrite are left in the plan
                    // and refused by the physical planner on purpose: "Unsupported predicate,
                    // subquery should not be decorrelated" (optimizer/src/scalar_subquery_to_join.rs
                    // scalar_subquery_where_not_eq), "errors rather than returning wrong rows"
                    // (sqllogictest/test_files/subquery.slt). The variant only succeeded because
                    // its optimizer proved the subquery's input empty and dropped it (e.g. a
                    // null-rejecting filter over an outer join folded to false). Whether folding
                    // gets there within max_passes is plan shape, not a correctness claim.
                    verdict.notes.push("unsupported_subquery_folded_away".into());
                } else if rule_variant && !base_failed {
                    // Removing a rule may leave a plan the physical planner cannot
                    // handle (e.g. un-decorrelated subqueries): not a correctness claim.
                    verdict.notes.push("rule_removed_error".into());
                } else {
                    verdict.sig = Some(format!("toggle__error_{}", v.name));
                    verdict.msg = if base_failed {
                        format!("[{}] base failed: {}; variant succeeded", v.name, short(e))
                    } else {
                        format!("[{}] base succeeded; variant failed: {}", v.name, short(e))
                    };
                    return verdict;
                }
            }
            (Outcome::Panic(..), _) => unreachable!(),
        }
    }
    if matches!(base, Outcome::Err(_)) && verdict.notes.iter().all(|n| n == "both_error") {
        verdict.skip = Some("both_error".into());
    }
    verdict
}
