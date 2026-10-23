// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The self-contained test case: tables (schema, data, partition layout), one query,
//! and the configurations to run it under. Serialized to `case.json` in findings.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ty {
    I64,
    I32,
    F64,
    Str,
    Bool,
}

impl Ty {
    pub fn sql(self) -> &'static str {
        match self {
            Ty::I64 => "BIGINT",
            Ty::I32 => "INT",
            Ty::F64 => "DOUBLE",
            Ty::Str => "VARCHAR",
            Ty::Bool => "BOOLEAN",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Col {
    pub name: String,
    pub ty: Ty,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Table {
    pub name: String,
    pub cols: Vec<Col>,
    /// row-major; `None` is NULL; values are in their SQL-literal-free text form
    /// (ints and floats as decimal text, "NaN" allowed, strings raw, bools "true"/"false").
    pub rows: Vec<Vec<Option<String>>>,
    /// number of rows in each MemTable partition (sums to rows.len())
    pub parts: Vec<usize>,
    /// max rows per RecordBatch inside a partition
    pub batch: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Variant {
    pub name: String,
    pub settings: Vec<(String, String)>,
    #[serde(default)]
    pub remove_rules: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Case {
    pub tables: Vec<Table>,
    pub query: String,
    /// the query's top-level ORDER BY covers every output column, so the row
    /// sequence (not just the multiset) is determined
    pub ordered: bool,
    /// the baseline configuration every variant is compared against
    pub base: Variant,
    pub variants: Vec<Variant>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Verdict {
    /// failure signature, if the case failed
    pub sig: Option<String>,
    pub msg: String,
    /// why the case was not judged (e.g. "both_error"), if it was not
    pub skip: Option<String>,
    /// per-variant skip reasons (counted in stats)
    #[serde(default)]
    pub notes: Vec<String>,
    /// baseline error text, when the baseline failed
    #[serde(default)]
    pub base_err: Option<String>,
}

pub fn sql_literal(ty: Ty, v: &Option<String>) -> String {
    match v {
        None => "NULL".into(),
        Some(s) => match ty {
            Ty::Str => format!("'{}'", s.replace('\'', "''")),
            Ty::F64 if s == "NaN" => "'NaN'::double".into(),
            _ => s.clone(),
        },
    }
}

/// A datafusion-cli script equivalent to the case (partition layout is not
/// reproducible from SQL; the harness binary is the authoritative repro).
pub fn to_sql_script(c: &Case) -> String {
    let mut out = String::new();
    for t in &c.tables {
        let cols: Vec<String> = t.cols.iter().map(|c| format!("{} {}", c.name, c.ty.sql())).collect();
        out += &format!("CREATE TABLE {} ({});\n", t.name, cols.join(", "));
        if !t.rows.is_empty() {
            let rows: Vec<String> = t
                .rows
                .iter()
                .map(|r| {
                    let vs: Vec<String> = r.iter().zip(&t.cols).map(|(v, c)| sql_literal(c.ty, v)).collect();
                    format!("({})", vs.join(", "))
                })
                .collect();
            out += &format!("INSERT INTO {} VALUES {};\n", t.name, rows.join(", "));
        }
    }
    for v in std::iter::once(&c.base).chain(&c.variants) {
        out += &format!("-- config {}\n", v.name);
        for (k, val) in &v.settings {
            out += &format!("SET {k} = {val};\n");
        }
        for r in &v.remove_rules {
            out += &format!("-- (remove optimizer rule {r}: ctx.remove_optimizer_rule(\"{r}\"))\n");
        }
        out += &format!("{};\n", c.query);
    }
    out
}
