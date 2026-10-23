// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! duckharness: optimizer-toggle differential + crash/assert oracle for DuckDB.
//!
//!   duckharness worker                         (internal: the DuckDB child process)
//!   duckharness search <seed> <seconds> <out>  (run.sh)
//!   duckharness repro <case.json>              (repro.sh; exit 1 reproduces, 0 not, 2 undecided)
//!   duckharness sample <seed> <n>              (print generated cases, for eyeballing)

mod gen;
mod worker;

use ephyra::{Config, GenExt, Run, Source};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant};
use worker::{Outcome, Req, Resp, Worker, ERROR_FATAL, ERROR_INTERNAL};

/// A case as plain SQL: what repro needs.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Rendered {
    pub setup: Vec<String>,
    /// statements run before the default-settings query (and still in effect for the variant)
    #[serde(default)]
    pub base: Vec<String>,
    pub query: String,
    /// statements that switch to the variant configuration
    pub variant: Vec<String>,
    /// top-level ORDER BY covers every column: compare in order
    pub ordered: bool,
}

pub fn render(c: &gen::Case) -> Rendered {
    Rendered {
        setup: gen::render_setup(&c.tables),
        base: gen::render_base(c),
        query: gen::render_q(&c.query),
        variant: gen::render_variant(&c.variant),
        ordered: c.query.order.is_some(),
    }
}

#[derive(Debug, Clone)]
pub struct Fail {
    /// failure class: one panic site per class
    pub sig: &'static str,
    /// for optdiff: which variant ("noopt", "multi", or the single disabled optimizer)
    pub tag: Option<String>,
    pub msg: String,
}

impl Fail {
    /// the finding signature: `<oracle>__<kind>`, kind refined by the toggled optimizer for optdiff
    pub fn signature(&self) -> String {
        match &self.tag {
            Some(t) => format!("{}_{}", self.sig, t),
            None => self.sig.to_string(),
        }
    }
}

/// A stable, low-cardinality name for an INTERNAL error: the source file stem of an assertion
/// ("plan_comparison_join"), else the leading words of the message up to the first value
/// ("vector_reference_used_on_vector").
fn crash_tag(msg: &str) -> String {
    let m = msg.strip_prefix("INTERNAL Error: ").unwrap_or(msg);
    if let Some(i) = m.find("Assertion triggered in file \"") {
        let rest = &m[i + 29..];
        let path = rest.split('"').next().unwrap_or("");
        let stem = path.rsplit('/').next().unwrap_or("").trim_end_matches(".cpp").trim_end_matches(".hpp");
        return stem.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    }
    let head: String = m.chars().take_while(|c| !matches!(c, '(' | '"' | '\'' | '\n' | '0'..='9' | '[' | '{')).collect();
    let words: Vec<String> = head
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(5)
        .map(|w| w.to_ascii_lowercase())
        .collect();
    let tag = if words.is_empty() { "other".to_string() } else { words.join("_") };
    // bounds/null checks in duckdb's checked containers (vector, unique_ptr) say nothing about where they
    // fired: add the class of the first stack frame outside the exception/check helpers
    if tag.starts_with("attempted_to") {
        if let Some(site) = throw_site(msg) {
            return format!("{tag}__{site}");
        }
    }
    tag
}

/// snake-cased class (or free function) name of the first frame of the stack trace in `msg` that is not
/// part of throwing the exception
fn throw_site(msg: &str) -> Option<String> {
    for line in msg.lines() {
        let Some(open) = line.find("(duckdb::") else { continue };
        let sym = &line[open + 9..];
        let name = &sym[..sym.find('(').unwrap_or(sym.len())];
        let name = name.split('<').next().unwrap_or(name);
        if name.contains("Exception") || name.starts_with("Throw") || name.starts_with("vector") || name.starts_with("unique_ptr")
            || name.starts_with("shared_ptr") || name.starts_with("optional_ptr")
        {
            continue;
        }
        let parts: Vec<&str> = name.split("::").collect();
        let base = if parts.len() >= 2 { parts[parts.len() - 2] } else { parts[0] };
        let mut out = String::new();
        for (i, c) in base.chars().enumerate() {
            if c.is_ascii_uppercase() && i > 0 {
                out.push('_');
            }
            if c.is_ascii_alphanumeric() { out.push(c.to_ascii_lowercase()) } else { out.push('_') }
        }
        return if out.is_empty() { None } else { Some(out) };
    }
    None
}

/// the DEBUG_VARIANTS tag when the variant is a debug setting
fn debug_tag(r: &Rendered) -> Option<&'static str> {
    let v = r.variant.first()?;
    gen::DEBUG_VARIANTS.iter().find(|d| d.1 == v).map(|d| d.0)
}

fn is_explain(r: &Rendered) -> bool {
    r.variant.iter().any(|v| v.contains("debug_verify_statement"))
}

/// the construct an EXPLAIN (SQL) round-trip case exercises: the rarest one in the (shrunk) query
fn explain_area(r: &Rendered) -> &'static str {
    let q = r.query.to_ascii_uppercase();
    let has = |w: &str| q.contains(w);
    if has(" OVER ") || has(" OVER(") {
        "window"
    } else if has(" UNION") || has(" INTERSECT") || has(" EXCEPT") {
        "setop"
    } else if has(" ANY (") || has(" ALL (") || has(" ANY(") || has(" ALL(") || has(" IN (SELECT") {
        "quantified"
    } else if has("EXISTS") {
        "exists"
    } else if has("(SELECT") {
        "subquery"
    } else if has("WITH ") {
        "cte"
    } else if has(" SEMI ") || has(" ANTI ") {
        "semi_anti"
    } else if has(" LEFT ") || has(" RIGHT ") || has(" FULL ") {
        "outer_join"
    } else if has("GROUP BY") || has("COUNT(") || has("SUM(") || has("MIN(") || has("MAX(") || has("AVG(") {
        "aggregate"
    } else if has("DISTINCT") {
        "distinct"
    } else if has(" LIMIT ") || has(" OFFSET ") {
        "limit"
    } else if has(" JOIN ") || has(", T") {
        "join"
    } else {
        "scan"
    }
}

/// a stable name for an error message: its first words, without the "<Kind> Error: " prefix, quoted
/// names/values and numbers ("The \"quantile\" argument in function \"quantile_disc\" must be a constant"
/// -> "the_argument_in_function_must_be")
fn message_words(msg: &str) -> String {
    let m = msg.lines().next().unwrap_or("");
    let m = m.split_once(": ").map_or(m, |x| x.1);
    let mut plain = String::new();
    let mut quote: Option<char> = None;
    for c in m.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None => plain.push(c),
        }
    }
    let words: Vec<String> = plain
        .split(|c: char| !c.is_ascii_alphabetic())
        .filter(|w| !w.is_empty())
        .take(6)
        .map(|w| w.to_ascii_lowercase())
        .collect();
    if words.is_empty() { "other".into() } else { words.join("_") }
}

/// error class names for `duckdb_error_type` values that mean the exported SQL does not rebind
fn rebind_error_kind(etype: i32) -> Option<&'static str> {
    Some(match etype {
        5 => "mismatch_type",
        11 => "not_implemented",
        12 => "expression",
        13 => "catalog",
        14 => "parser",
        15 => "planner",
        22 => "syntax",
        24 => "binder",
        26 => "optimizer",
        _ => return None,
    })
}

fn variant_tag(r: &Rendered) -> &'static str {
    let v = r.variant.join(";");
    if v.contains("enable_optimizer=false") {
        return "noopt";
    }
    let names: Vec<&str> = v.split('\'').nth(1).unwrap_or("").split(',').collect();
    if names.len() == 1 {
        if let Some(n) = gen::OPTIMIZERS.iter().find(|n| **n == names[0]) {
            return n;
        }
    }
    "multi"
}

#[derive(Default, Debug, Serialize)]
pub struct Stats {
    both_ok: u64,
    both_err: u64,
    /// one side errors (non-internal), the other succeeds: not a finding (different plans may evaluate
    /// a failing cast/arithmetic on rows the other plan filters out; test/configs/verify_statement_explain_sql.json)
    err_mismatch: u64,
    /// EXPLAIN (SQL) renders an internal aggregate (e.g. arg_min_nulls_last from TopN window elimination) that
    /// debug_verify_aggregate_state_export cannot export: a documented gap, not a finding
    skip_state_export: u64,
    setup_err: u64,
    variant_timeout: u64,
    big_results: u64,
    /// wall time spent per phase, ms (throughput diagnosis)
    ms_setup: u64,
    ms_default: u64,
    ms_variant: u64,
    /// error classes of the default-settings query when both sides error (first 45 chars)
    err_kinds: BTreeMap<String, u64>,
}

const SETUP_LIMIT: Duration = Duration::from_secs(30);
const QUERY_LIMIT: Duration = Duration::from_secs(30);
const CAP: u64 = 2000;

fn run1(w: &mut Worker, open: bool, sql: &str, limit: Duration) -> Outcome {
    w.request(&Req { open, sql: sql.into(), cap: CAP }, limit)
}

fn internal(r: &Resp) -> Option<Fail> {
    if r.ok {
        return None;
    }
    if r.etype == ERROR_INTERNAL {
        let sig = if r.msg.contains("Assertion triggered") { "crash__assert" } else { "crash__internal" };
        return Some(Fail { sig, tag: Some(crash_tag(&r.msg)), msg: r.msg.clone() });
    }
    if r.etype == ERROR_FATAL {
        return Some(Fail { sig: "crash__fatal", tag: None, msg: r.msg.clone() });
    }
    None
}

fn show(r: &Resp) -> String {
    if !r.ok {
        return format!("error({}): {}", r.etype, r.msg);
    }
    let mut s = format!("{} rows, names {:?}, types {:?}", r.nrows, r.names, r.types);
    if let Some(rows) = &r.rows {
        let mut v: Vec<String> = rows.iter().map(|r| format!("{:?}", r)).collect();
        v.truncate(20);
        s.push_str(&format!("\n    {}", v.join("\n    ")));
    }
    s
}

/// One case. Err = a finding.
pub fn check(w: &mut Worker, r: &Rendered, st: &mut Stats, fresh: bool) -> Result<(), Fail> {
    let res = check_inner(w, r, st, fresh);
    if res.is_err() {
        w.dirty = true;
    }
    res
}

fn check_inner(w: &mut Worker, r: &Rendered, st: &mut Stats, fresh: bool) -> Result<(), Fail> {
    let died = |how: String, what: &str| Fail { sig: "crash__signal", tag: None, msg: format!("worker died during {what}: {how}") };
    let t = Instant::now();
    // Opening a database costs ~28 ms; statements ~1 ms. Reuse the database and drop the tables,
    // except after an INTERNAL error (it invalidates the database) or in repro mode (fresh=true).
    let mut prep = vec!["SET threads=2".to_string()];
    if !(w.dirty || fresh) {
        prep = vec!["DROP TABLE IF EXISTS t0".into(), "DROP TABLE IF EXISTS t1".into(), "DROP TABLE IF EXISTS t2".into()];
    }
    prep.push("SET enable_optimizer=true".into());
    prep.push("SET debug_verify_statement='none'".into());
    prep.push("SET disabled_optimizers=''".into());
    let open = w.dirty || fresh;
    w.dirty = false;
    // undo the debug settings of the previous case on a reused database
    if !open {
        prep.extend(std::mem::take(&mut w.reset));
    }
    w.reset = r.base.iter().chain(r.variant.iter()).filter_map(|s| gen::reset_of(s)).map(String::from).collect();
    for (i, p) in prep.iter().enumerate() {
        match run1(w, open && i == 0, p, SETUP_LIMIT) {
            Outcome::Done(x) if x.ok => {}
            Outcome::Done(x) => return Err(Fail { sig: "harness__prep", tag: None, msg: format!("{p}: {}", x.msg) }),
            Outcome::Died(h) => return Err(died(h, "open")),
            Outcome::Timeout => return Err(Fail { sig: "crash__hang", tag: None, msg: "open timed out".into() }),
        }
    }
    for s in &r.setup {
        match run1(w, false, s, SETUP_LIMIT) {
            Outcome::Done(resp) => {
                if let Some(f) = internal(&resp) {
                    return Err(f);
                }
                if !resp.ok {
                    st.setup_err += 1;
                    return Ok(());
                }
            }
            Outcome::Died(h) => return Err(died(h, "setup")),
            Outcome::Timeout => {
                st.setup_err += 1;
                return Ok(());
            }
        }
    }
    for s in &r.base {
        match run1(w, false, s, SETUP_LIMIT) {
            Outcome::Done(x) if x.ok => {}
            Outcome::Done(x) => return Err(Fail { sig: "harness__variant", tag: None, msg: format!("{s}: {}", x.msg) }),
            Outcome::Died(h) => return Err(died(h, "base setting")),
            Outcome::Timeout => return Err(Fail { sig: "crash__hang", tag: None, msg: "SET timed out".into() }),
        }
    }
    st.ms_setup += t.elapsed().as_millis() as u64;
    let t = Instant::now();
    let a = match run1(w, false, &r.query, QUERY_LIMIT) {
        Outcome::Done(x) => x,
        Outcome::Died(h) => return Err(died(h, "the default-settings query")),
        Outcome::Timeout => return Err(Fail { sig: "crash__hang", tag: None, msg: format!("default-settings query exceeded {QUERY_LIMIT:?}") }),
    };
    st.ms_default += t.elapsed().as_millis() as u64;
    if let Ok(ms) = std::env::var("DH_SLOW_MS") {
        if t.elapsed().as_millis() as u64 > ms.parse::<u64>().unwrap_or(1000) {
            eprintln!("SLOW {:?}\n  setup: {:?}\n  Q: {}\n", t.elapsed(), r.setup, r.query);
        }
    }
    if let Some(f) = internal(&a) {
        return Err(f);
    }
    for s in &r.variant {
        match run1(w, false, s, SETUP_LIMIT) {
            Outcome::Done(x) if x.ok => {}
            Outcome::Done(x) => return Err(Fail { sig: "harness__variant", tag: None, msg: format!("{s}: {}", x.msg) }),
            Outcome::Died(h) => return Err(died(h, "variant setting")),
            Outcome::Timeout => return Err(Fail { sig: "crash__hang", tag: None, msg: "SET timed out".into() }),
        }
    }
    // the variant plan may be legitimately much slower (no join ordering: cross products)
    let vlimit = Duration::from_secs(5).max(t.elapsed() * 10).min(QUERY_LIMIT);
    let t = Instant::now();
    let b = match run1(w, false, &r.query, vlimit) {
        Outcome::Done(x) => x,
        Outcome::Died(h) => return Err(died(h, "the variant query")),
        Outcome::Timeout => {
            st.variant_timeout += 1;
            return Ok(());
        }
    };
    st.ms_variant += t.elapsed().as_millis() as u64;
    if let Some(f) = internal(&b) {
        return Err(f);
    }
    match (a.ok, b.ok) {
        (false, false) => {
            st.both_err += 1;
            let k: String = a.msg.chars().take(45).collect();
            if let Ok(pat) = std::env::var("DH_SHOW_ERR") {
                if a.msg.contains(&pat) {
                    eprintln!("ERR {}\n  setup: {:?}\n  Q: {}\n", a.msg.lines().next().unwrap_or(""), r.setup, r.query);
                }
            }
            *st.err_kinds.entry(k).or_default() += 1;
            return Ok(());
        }
        (true, true) => st.both_ok += 1,
        (true, false)
            if is_explain(r)
                && r.base.iter().any(|s| s.contains("debug_verify_aggregate_state_export=true"))
                && b.msg.contains("does not have a state type callback defined") =>
        {
            // test/configs/verify_aggregate_state_export.json skips arg_min_max_nulls_last.test et al.: "FIXME:
            // aggregate states that own memory cannot be exported and have no state type callback (arg_min/arg_max(N),
            // min/max(N), ...)". The default query binds a window (rewritten to such an aggregate only in the
            // optimizer, after the verification rewrite); the exported SQL binds the aggregate itself.
            st.skip_state_export += 1;
            return Ok(());
        }
        (true, false) if is_explain(r) && rebind_error_kind(b.etype).is_some() => {
            // "The generated query preserves SQL semantics when rebound under the same catalog, settings,
            // extensions" (src/planner/sql_export/README.md); unsupported shapes fall back to the original
            // query under explain_sql, so an error that is not a runtime one means the export is broken
            let kind = rebind_error_kind(b.etype).unwrap();
            let words = message_words(&b.msg);
            return Err(Fail {
                sig: "explainsql__error",
                tag: Some(format!("{kind}_{words}")),
                msg: format!("EXPLAIN (SQL) export does not run; base {:?}\n  default: {}\n  exported: {}", r.base, show(&a), show(&b)),
            });
        }
        _ => {
            st.err_mismatch += 1;
            return Ok(());
        }
    }
    if a.nrows > CAP {
        st.big_results += 1;
    }
    let ctx = || format!("variant {:?}\n  default: {}\n  variant: {}", r.variant, show(&a), show(&b));
    if let Some(tag) = debug_tag(r) {
        if a.names != b.names || a.types != b.types {
            return Err(Fail { sig: "debugdiff__schema", tag: Some(tag.into()), msg: format!("result schema differs; {}", ctx()) });
        }
        let same = a.nrows == b.nrows && if r.ordered { a.ohash == b.ohash } else { a.mhash == b.mhash };
        if !same {
            return Err(Fail { sig: "debugdiff__rows", tag: Some(tag.into()), msg: format!("rows differ ({}); base {:?}; {}", if r.ordered { "ordered" } else { "multiset" }, r.base, ctx()) });
        }
        return Ok(());
    }
    if is_explain(r) {
        let ctx = || format!("base {:?}\n  default: {}\n  exported SQL: {}", r.base, show(&a), show(&b));
        if a.names != b.names || a.types != b.types {
            return Err(Fail { sig: "explainsql__schema", tag: Some(explain_area(r).into()), msg: format!("EXPLAIN (SQL) changes the result schema; {}", ctx()) });
        }
        let same = a.nrows == b.nrows && if r.ordered { a.ohash == b.ohash } else { a.mhash == b.mhash };
        if !same {
            return Err(Fail { sig: "explainsql__rows", tag: Some(explain_area(r).into()), msg: format!("EXPLAIN (SQL) changes the rows ({}); {}", if r.ordered { "ordered" } else { "multiset" }, ctx()) });
        }
        return Ok(());
    }
    if a.names != b.names || a.types != b.types {
        return Err(Fail { sig: "optdiff__schema", tag: Some(variant_tag(r).to_string()), msg: format!("result schema differs; {}", ctx()) });
    }
    let same = a.nrows == b.nrows && if r.ordered { a.ohash == b.ohash } else { a.mhash == b.mhash };
    if !same {
        return Err(Fail { sig: "optdiff__rows", tag: Some(variant_tag(r).to_string()), msg: format!("rows differ ({}); {}", if r.ordered { "ordered" } else { "multiset" }, ctx()) });
    }
    Ok(())
}

/// The property: one panic site per signature so shrinking never trades one failure for another.
fn property(w: &mut Worker, r: &Rendered, st: &mut Stats) {
    if let Err(f) = check(w, r, st, false) {
        match f.sig {
            "crash__signal" => panic!("SIG:crash__signal|{}", f.msg),
            "crash__hang" => panic!("SIG:crash__hang|{}", f.msg),
            "crash__assert" => panic!("SIG:crash__assert|{}", f.msg),
            "crash__internal" => panic!("SIG:crash__internal|{}", f.msg),
            "crash__fatal" => panic!("SIG:crash__fatal|{}", f.msg),
            "optdiff__schema" => panic!("SIG:optdiff__schema|{}", f.msg),
            "optdiff__rows" => panic!("SIG:optdiff__rows|{}", f.msg),
            "explainsql__schema" => panic!("SIG:explainsql__schema|{}", f.msg),
            "explainsql__rows" => panic!("SIG:explainsql__rows|{}", f.msg),
            "explainsql__error" => panic!("SIG:explainsql__error|{}", f.msg),
            "debugdiff__schema" => panic!("SIG:debugdiff__schema|{}", f.msg),
            "debugdiff__rows" => panic!("SIG:debugdiff__rows|{}", f.msg),
            _ => panic!("SIG:harness__other|{}", f.msg),
        }
    }
}

fn case_gen() -> impl ephyra::Gen<Value = gen::Case> {
    (|g: &mut Source| gen::gen_case(g)).with_shrink(gen::shrink_case)
}

fn sql_script(r: &Rendered) -> String {
    let mut s = String::from("-- setup\n");
    for x in &r.setup {
        s.push_str(x);
        s.push_str(";\n");
    }
    s.push_str("SET threads=2;\n");
    for x in &r.base {
        s.push_str(x);
        s.push_str(";\n");
    }
    s.push_str("-- default settings:\n");
    s.push_str(&r.query);
    s.push_str(";\n-- variant:\n");
    for x in &r.variant {
        s.push_str(x);
        s.push_str(";\n");
    }
    s.push_str(&r.query);
    s.push_str(";\n");
    s
}

#[derive(Serialize, Deserialize)]
struct CaseFile {
    signature: String,
    case: Rendered,
}

fn write_finding(out: &str, n: usize, f: &Fail, r: &Rendered, seed: &str) {
    let sig = f.signature();
    let dir = format!("{out}/findings/{sig}/{n}");
    std::fs::create_dir_all(&dir).unwrap();
    let oracle = f.sig.split("__").next().unwrap();
    let fj = serde_json::json!({"signature": sig, "oracle": oracle, "message": f.msg, "seed": seed});
    std::fs::write(format!("{dir}/finding.json"), serde_json::to_string_pretty(&fj).unwrap()).unwrap();
    let cf = CaseFile { signature: sig.clone(), case: r.clone() };
    std::fs::write(format!("{dir}/case.json"), serde_json::to_string_pretty(&cf).unwrap()).unwrap();
    std::fs::write(format!("{dir}/case.sql"), sql_script(r)).unwrap();
    let repro = "#!/bin/bash\n# usage: repro.sh <build-dir>; exit 1 = reproduces, 0 = does not, 2 = cannot decide\n\
B=$(realpath \"$1\") || exit 2\ncd \"$(dirname \"$0\")\" || exit 2\n[ -x \"$B/duckharness\" ] || exit 2\n\
LD_LIBRARY_PATH=\"$B/cmake/src\" timeout 600 \"$B/duckharness\" repro case.json\n";
    let p = format!("{dir}/repro.sh");
    std::fs::write(&p, repro).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn write_stats(out: &str, cases: u64, failures: u64, st: &Stats, sigs: &BTreeMap<String, u64>, t0: Instant) {
    let el = t0.elapsed().as_secs_f64();
    let mut v = serde_json::to_value(st).unwrap();
    let m = v.as_object_mut().unwrap();
    m.insert("cases".into(), cases.into());
    m.insert("failures".into(), failures.into());
    m.insert("seconds".into(), (el as u64).into());
    m.insert("cases_per_s".into(), serde_json::json!(((cases as f64 / el.max(0.001)) * 10.0).round() / 10.0));
    m.insert("failures_by_signature".into(), serde_json::to_value(sigs).unwrap());
    let tmp = format!("{out}/stats.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    std::fs::rename(tmp, format!("{out}/stats.json")).unwrap();
}

fn sig_of(msg: &str) -> String {
    msg.strip_prefix("SIG:").and_then(|m| m.split('|').next()).unwrap_or("harness__panic").to_string()
}

fn search(seed: u64, secs: u64, out: &str) {
    std::fs::create_dir_all(format!("{out}/findings")).unwrap();
    let t0 = Instant::now();
    let deadline = t0 + Duration::from_secs(secs);
    let w = Rc::new(RefCell::new(Worker::new()));
    let st = Rc::new(RefCell::new(Stats::default()));
    let mut cases = 0u64;
    let mut failures = 0u64;
    let mut sigs: BTreeMap<String, u64> = BTreeMap::new();
    let mut last_stats = Instant::now();
    let mut round = 0u64;
    write_stats(out, 0, 0, &st.borrow(), &sigs, t0);
    while Instant::now() < deadline {
        round += 1;
        let rseed = seed.wrapping_mul(1_000_003).wrapping_add(round);
        let left = deadline.saturating_duration_since(Instant::now());
        // rounds of at most 60 s, so the shrink time limit can respect the overall deadline
        let round_budget = left.min(Duration::from_secs(60));
        let shrink_ms = (left - round_budget).as_millis().clamp(5_000, 60_000) as u64;
        let mut config = Config::default().seed(rseed).tests(u64::MAX / 2).budget(round_budget).no_regressions().name("optdiff");
        config.shrink.timeout_ms = Some(shrink_ms);
        let (w2, st2) = (w.clone(), st.clone());
        let mut run = Run::new(&config, case_gen(), move |c: gen::Case| {
            let r = render(&c);
            property(&mut w2.borrow_mut(), &r, &mut st2.borrow_mut());
        });
        let mut failed = false;
        while run.wants_more() && Instant::now() < deadline {
            let Some(input) = run.draw() else { continue };
            cases += 1;
            let tested = run.test(&input);
            if last_stats.elapsed() > Duration::from_secs(2) {
                write_stats(out, cases, failures, &st.borrow(), &sigs, t0);
                last_stats = Instant::now();
            }
            if tested.failed() {
                failed = true;
                break;
            }
        }
        if !failed {
            continue;
        }
        failures += 1;
        let orig = run.failure().cloned().unwrap();
        // what failed, on the original input
        let mut scratch = Stats::default();
        let first = check(&mut w.borrow_mut(), &render(&orig), &mut scratch, true).err();
        let Some(first) = first else {
            *sigs.entry("flaky".into()).or_default() += 1;
            continue;
        };
        // shrink unless this signature is already capped (a single-optimizer optdiff variant cannot
        // change its signature by shrinking; other variants can, so they are always shrunk)
        let capped = |sigs: &BTreeMap<String, u64>, s: &str| sigs.get(s).copied().unwrap_or(0) >= 3;
        let pre = first.signature();
        let may_change = matches!(first.tag.as_deref(), Some("noopt") | Some("multi")) || first.sig.starts_with("explainsql");
        if capped(&sigs, &pre) && !may_change {
            *sigs.entry(pre).or_default() += 1;
            write_stats(out, cases, failures, &st.borrow(), &sigs, t0);
            continue;
        }
        // hangs cost the whole time limit per shrink step: report them unshrunk
        let shrunk = if first.sig == "crash__hang" { orig.clone() } else { run.shrink().cloned().unwrap_or(orig.clone()) };
        let mut r = render(&shrunk);
        let mut f = check(&mut w.borrow_mut(), &r, &mut scratch, true).err();
        if f.as_ref().map(|f| f.sig) != Some(first.sig) {
            r = render(&orig);
            f = Some(first.clone());
        }
        let f = f.unwrap();
        // a finding must reproduce again on a fresh database, else it is counted as flaky and dropped
        let again = check(&mut w.borrow_mut(), &r, &mut scratch, true).err().map(|g| g.signature());
        if again.as_deref() != Some(f.signature().as_str()) {
            *sigs.entry(format!("flaky_{}", f.sig)).or_default() += 1;
            write_stats(out, cases, failures, &st.borrow(), &sigs, t0);
            continue;
        }
        let count = sigs.entry(f.signature()).or_default();
        *count += 1;
        if *count <= 3 {
            write_finding(out, *count as usize, &f, &r, &format!("search seed {seed} round {round} (ephyra seed {rseed})"));
        }
        write_stats(out, cases, failures, &st.borrow(), &sigs, t0);
    }
    write_stats(out, cases, failures, &st.borrow(), &sigs, t0);
    let _ = sig_of;
}

fn repro(path: &str) -> i32 {
    let Ok(text) = std::fs::read_to_string(path) else { return 2 };
    let Ok(cf) = serde_json::from_str::<CaseFile>(&text) else { return 2 };
    let mut w = Worker::new();
    for attempt in 0..3 {
        let mut st = Stats::default();
        match check(&mut w, &cf.case, &mut st, true) {
            Err(f) if f.signature() == cf.signature => {
                println!("reproduced {} (attempt {}): {}", f.signature(), attempt + 1, f.msg);
                return 1;
            }
            Err(f) => println!("different failure {}: {}", f.signature(), f.msg),
            Ok(()) => println!("passed: {:?}", st),
        }
    }
    0
}

fn sample(seed: u64, n: u64) {
    for i in 0..n {
        let mut g = Source::new(seed.wrapping_add(i), 20);
        let c = gen::gen_case(&mut g);
        let r = render(&c);
        println!("{}\n", sql_script(&r));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info.payload().downcast_ref::<String>().map(|s| s.as_str()).unwrap_or("");
        if !msg.starts_with("SIG:") {
            prev(info);
        }
    }));
    match args.get(1).map(|s| s.as_str()) {
        Some("worker") => worker::worker_main(),
        Some("search") => search(args[2].parse().unwrap(), args[3].parse().unwrap(), &args[4]),
        Some("repro") => std::process::exit(repro(&args[2])),
        Some("sample") => sample(args[2].parse().unwrap(), args[3].parse().unwrap()),
        _ => {
            eprintln!("usage: duckharness worker|search <seed> <secs> <out>|repro <case.json>|sample <seed> <n>");
            std::process::exit(2);
        }
    }
}
