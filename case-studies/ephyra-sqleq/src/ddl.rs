// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! A model's schema as the plain DDL sqleq reads: column types, NOT NULL,
//! the primary key and plain UNIQUE indexes (as UNIQUE constraints), none
//! of the physical choices (storage, compression, identity, partitioning,
//! other indexes), which change no query's result.

use ephyra_postgresql::model::{Model, Table};

pub fn create_table(t: &Table) -> String {
    let mut parts: Vec<String> = t
        .columns
        .iter()
        .map(|c| {
            // An identity column is NOT NULL.
            let nn = c.not_null || c.identity || t.key.contains(&c.id);
            format!("c{} {}{}", c.id, c.ty.name(), if nn { " NOT NULL" } else { "" })
        })
        .collect();
    let list = |cs: &[u32]| cs.iter().map(|c| format!("c{c}")).collect::<Vec<_>>().join(", ");
    if !t.key.is_empty() {
        parts.push(format!("PRIMARY KEY ({})", list(&t.key)));
    }
    for (cols, nulls_not_distinct) in t.uniques().into_iter().skip(usize::from(!t.key.is_empty())) {
        // NULLS NOT DISTINCT is a stronger constraint sqleq has no word for:
        // leaving it out only weakens what the prover may assume.
        let _ = nulls_not_distinct;
        parts.push(format!("UNIQUE ({})", list(&cols)));
    }
    format!("CREATE TABLE t{} ({})", t.id, parts.join(", "))
}

pub fn schema(m: &Model) -> String {
    m.tables.iter().map(|t| format!("{};\n", create_table(t))).collect()
}

/// An sqleq input file: the schema and the two queries.
pub fn pair_file(m: &Model, a: &str, b: &str, note: &str) -> String {
    format!("-- {note}\n{}{a};\n{b};\n", schema(m))
}
