// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! One-step variants: a seed with one rule applied at one place. An
//! unproved one-step pair names the rule sqleq cannot follow, and keeping
//! that one application while the seed shrinks gives small pairs.
//!
//! The seed's term goes into an e-graph unsaturated (one node per class,
//! identical subterms shared). Each rule is searched; per match, a copy of
//! the e-graph has that one substitution applied, and a term is read out
//! that takes a new node at the matched class (once) and the seed's own
//! node everywhere else.
//!
//! The relational rewrites ([`crate::relational`]) are one-step variants
//! too, appended under their own rule names (`rel-..`).

use std::collections::HashMap;

use egg::{EGraph, Id, Language, RecExpr, SearchMatches};
use ephyra_postgresql::expr::{Query, Schema};

use crate::lang::{decode, encode, rules, Sql, Types};

/// (rule name, variant), deduplicated by SQL, every one valid on `schema`.
pub fn one_step(seed: &Query, schema: &Schema) -> Vec<(String, Query)> {
    let enc = encode(seed);
    let mut eg: EGraph<Sql, Types> = EGraph::default();
    let mut class_of: Vec<Id> = Vec::with_capacity(enc.term.as_ref().len());
    for n in enc.term.as_ref() {
        let n = n.clone().map_children(|c| class_of[usize::from(c)]);
        class_of.push(eg.add(n));
    }
    eg.rebuild();
    let root = *class_of.last().unwrap();
    // The seed's node of each class, children as classes.
    let original: HashMap<Id, Sql> = enc.term.as_ref().iter().enumerate().map(|(i, n)| (class_of[i], n.clone().map_children(|c| class_of[usize::from(c)]))).collect();
    let seed_sql = seed.sql();
    let mut out: Vec<(String, Query)> = Vec::new();
    for rule in rules() {
        for m in rule.search(&eg) {
            for subst in &m.substs {
                let mut g = eg.clone();
                let single = SearchMatches { eclass: m.eclass, substs: vec![subst.clone()], ast: m.ast.clone() };
                if rule.apply(&mut g, &[single]).is_empty() {
                    continue;
                }
                g.rebuild();
                let mut term = RecExpr::default();
                let mut used = false;
                let target = g.find(m.eclass);
                if read(&g, &original, root, target, &mut used, &mut term, 0).is_none() || !used {
                    continue;
                }
                let Ok(q) = decode(&term, &enc) else { continue };
                let sql = q.sql();
                if sql != seed_sql && q.valid(schema) && !out.iter().any(|(_, o)| o.sql() == sql) {
                    out.push((rule.name.to_string(), q));
                }
            }
        }
    }
    for (rule, q) in crate::relational::relational(seed, schema) {
        if !out.iter().any(|(_, o)| o.sql() == q.sql()) {
            out.push((rule, q));
        }
    }
    out
}

/// Read a term from `class`: at `target` (first visit) a node that is not
/// the seed's, elsewhere the seed's node (a class the rule made has only
/// its own).
fn read(g: &EGraph<Sql, Types>, original: &HashMap<Id, Sql>, class: Id, target: Id, used: &mut bool, term: &mut RecExpr<Sql>, depth: usize) -> Option<Id> {
    if depth > 200 {
        return None;
    }
    let c = g.find(class);
    let canon = |n: &Sql| n.clone().map_children(|x| g.find(x));
    let seed_node = original.iter().find(|(k, _)| g.find(**k) == c).map(|(_, n)| canon(n));
    let node = if c == target && !*used {
        *used = true;
        g[c].nodes.iter().find(|n| Some(canon(n)) != seed_node)?.clone()
    } else {
        match seed_node {
            Some(n) => n,
            None => g[c].nodes.first()?.clone(),
        }
    };
    let mut kids = Vec::new();
    for &ch in node.children() {
        kids.push(read(g, original, ch, target, used, term, depth + 1)?);
    }
    let mut it = kids.into_iter();
    Some(term.add(node.map_children(|_| it.next().unwrap())))
}
