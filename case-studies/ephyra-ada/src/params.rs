// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! URLSearchParams cases: [`ParamsCase`] (an init string and operations),
//! run on ada ([`ada`]) and on whatwg-url ([`reference`]), and drawn by
//! [`params_cases`].
//!
//! Strings that are not valid UTF-8 can come out of ada (percent-decoding
//! yields arbitrary bytes); [`text`] keeps such bytes visible as `\xNN`
//! rather than replacing them, so they show up as differences from the
//! standard, which decodes them to U+FFFD.

use std::ops::ControlFlow;

use ephyra::{Gen, Source};
use serde_json::{json, Value};

use crate::gen;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    Append,
    Delete,
    DeleteValue,
    Set,
    Sort,
    Get,
    GetAll,
    Has,
    HasValue,
    /// `keys()`, `values()` and `entries()`: a fresh iterator, run to the
    /// end (ada's `get_keys`/`get_values`/`get_entries` iterators, which
    /// the range-for behind [`ParamsStep::entries`] does not use).
    Keys,
    Values,
    Entries,
    /// `next()` on one entries iterator made right after the constructor,
    /// so the operations in between change the list it walks. The standard
    /// defines that walk (an index into the live list, not latched at the
    /// end); ada documents its iterators as invalidated by a change, but
    /// they index the live list too.
    Next,
}

impl Op {
    pub const ALL: [Op; 13] = [
        Op::Append,
        Op::Delete,
        Op::DeleteValue,
        Op::Set,
        Op::Sort,
        Op::Get,
        Op::GetAll,
        Op::Has,
        Op::HasValue,
        Op::Keys,
        Op::Values,
        Op::Entries,
        Op::Next,
    ];
}

/// `new URLSearchParams(init)`, then each operation with its key and value
/// (ignored by the operations that take fewer arguments).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ParamsCase {
    pub init: String,
    pub ops: Vec<(Op, String, String)>,
}

/// After the constructor or an operation: its result (nothing, the value
/// got, the values, `true` or `false`), `size`, `toString()` and the entries
/// as alternating keys and values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamsStep {
    pub result: Vec<String>,
    pub size: String,
    pub serialized: String,
    pub entries: Vec<String>,
}

/// `bytes` as a string, invalid UTF-8 bytes as `\xNN`.
pub fn text(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.utf8_chunks() {
        out.push_str(chunk.valid());
        for b in chunk.invalid() {
            out.push_str(&format!("\\x{b:02X}"));
        }
    }
    out
}

extern "C" {
    fn ea_params(bytes: *const u8, len: usize, out: *mut *mut u8) -> usize;
    fn ea_free(p: *mut u8);
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

/// The transcript of `case` in ada.
pub fn ada(case: &ParamsCase) -> Vec<ParamsStep> {
    let mut bytes = Vec::new();
    put_str(&mut bytes, &case.init);
    bytes.extend_from_slice(&(case.ops.len() as u32).to_le_bytes());
    for (op, k, v) in &case.ops {
        bytes.push(Op::ALL.iter().position(|o| o == op).unwrap() as u8);
        put_str(&mut bytes, k);
        put_str(&mut bytes, v);
    }
    let mut out = std::ptr::null_mut();
    // SAFETY: `bytes` is live for the call; the shim writes a malloc'd
    // buffer of the returned length to `out`, copied and then freed.
    let t = crate::ada::locked(|| unsafe {
        let len = ea_params(bytes.as_ptr(), bytes.len(), &mut out);
        let t = std::slice::from_raw_parts(out, len).to_vec();
        ea_free(out);
        t
    });
    let mut r = Reader(&t);
    let mut steps = Vec::new();
    while !r.0.is_empty() {
        let result = r.list();
        let size = r.str();
        let serialized = r.str();
        let entries = r.list();
        steps.push(ParamsStep { result, size, serialized, entries });
    }
    steps
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn u32(&mut self) -> usize {
        let v = u32::from_le_bytes(self.0[..4].try_into().unwrap());
        self.0 = &self.0[4..];
        v as usize
    }

    fn str(&mut self) -> String {
        let n = self.u32();
        let s = text(&self.0[..n]);
        self.0 = &self.0[n..];
        s
    }

    fn list(&mut self) -> Vec<String> {
        (0..self.u32()).map(|_| self.str()).collect()
    }
}

fn name(op: Op) -> &'static str {
    match op {
        Op::Append => "append",
        Op::Delete => "delete",
        Op::DeleteValue => "deleteValue",
        Op::Set => "set",
        Op::Sort => "sort",
        Op::Get => "get",
        Op::GetAll => "getAll",
        Op::Has => "has",
        Op::HasValue => "hasValue",
        Op::Keys => "keys",
        Op::Values => "values",
        Op::Entries => "entries",
        Op::Next => "next",
    }
}

/// The transcript of `case` in whatwg-url's URLSearchParams.
pub fn reference(case: &ParamsCase) -> Vec<ParamsStep> {
    let ops: Vec<Value> = case.ops.iter().map(|(o, k, v)| json!([name(*o), k, v])).collect();
    let v = crate::reference::request(&json!({ "params": case.init, "ops": ops }));
    let strings =
        |v: &Value| v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_owned()).collect();
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| ParamsStep {
            result: strings(&s["result"]),
            size: s["size"].as_u64().unwrap().to_string(),
            serialized: s["serialized"].as_str().unwrap().to_owned(),
            entries: strings(&s["entries"]),
        })
        .collect()
}

/// Keys and values: a small pool, so operations meet the keys the init
/// string and earlier operations made, and free text from the URL grammar.
fn key(g: &mut Source) -> String {
    if g.bool_with(0.6) {
        gen::pick_str(
            g,
            &[
                "a",
                "b",
                "A",
                "",
                "a b",
                "a+b",
                "é",
                "e\u{301}",
                "\u{1F600}",
                "\u{FFFD}",
                "\u{E000}",
                "\u{FFFF}",
                "%41",
                "%",
                "&",
                "=",
                "?",
                "a=b",
                "\u{1}",
                "\u{0}",
                "~",
            ],
        )
    } else {
        gen::text(g, 6)
    }
}

/// An init string: pairs of keys and values, written as a query (with `+`
/// for space and percent escapes, valid, invalid or decoding to invalid
/// UTF-8), joined by `&`, sometimes after a `?`.
fn init(g: &mut Source) -> String {
    let max = gen::sized(g, 10).min(8);
    let n = gen::few(g, max);
    let mut parts = Vec::new();
    for _ in 0..n {
        let mut part = key(g);
        if g.bool_with(0.15) {
            part.push_str(&gen::pick_str(
                g,
                &["%FF", "%C3", "%C3%28", "%E2%82", "%F0%9F%98", "%ED%A0%80", "%80", "+"],
            ));
        }
        if g.bool_with(0.8) {
            part.push('=');
            part.push_str(&key(g));
        }
        parts.push(part);
    }
    let sep = *g.pick(&["&", "&", "&", "&&", "&="]);
    let mut out = parts.join(sep);
    if g.bool_with(0.2) {
        out.insert(0, '?');
    }
    out
}

fn op(g: &mut Source) -> (Op, String, String) {
    (*g.pick(&Op::ALL), key(g), key(g))
}

/// Search-params cases: see the module documentation. Shrinking drops
/// operations, then deletes characters from the strings and simplifies
/// them; mutation adds, replaces or drops an operation or redraws the init.
pub struct ParamsCases;

impl ParamsCases {
    fn strings(case: &ParamsCase) -> usize {
        1 + 2 * case.ops.len()
    }

    fn get(case: &ParamsCase, at: usize) -> &str {
        match at {
            0 => &case.init,
            k if k % 2 == 1 => &case.ops[(k - 1) / 2].1,
            k => &case.ops[(k - 2) / 2].2,
        }
    }

    fn with(case: &ParamsCase, at: usize, s: String) -> ParamsCase {
        let mut out = case.clone();
        match at {
            0 => out.init = s,
            k if k % 2 == 1 => out.ops[(k - 1) / 2].1 = s,
            k => out.ops[(k - 2) / 2].2 = s,
        }
        out
    }
}

impl Gen for ParamsCases {
    type Value = ParamsCase;

    fn generate(&self, g: &mut Source) -> ParamsCase {
        let max = gen::sized(g, 10).min(10);
        let n = gen::few(g, max);
        ParamsCase { init: init(g), ops: (0..n).map(|_| op(g)).collect() }
    }

    fn shrink(
        &self,
        case: &ParamsCase,
        visit: &mut dyn FnMut(ParamsCase) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        for k in (0..case.ops.len()).rev() {
            let mut out = case.clone();
            out.ops.remove(k);
            visit(out)?;
        }
        for at in 0..Self::strings(case) {
            gen::deletions(Self::get(case, at), &mut |t| visit(Self::with(case, at, t)))?;
        }
        for at in 0..Self::strings(case) {
            gen::simplifications(Self::get(case, at), &mut |t| visit(Self::with(case, at, t)))?;
        }
        ControlFlow::Continue(())
    }

    fn mutate(&self, case: &ParamsCase, g: &mut Source) -> ParamsCase {
        let mut out = case.clone();
        match g.int(0u32..4) {
            0 => {
                let at = g.int(0..=out.ops.len());
                out.ops.insert(at, op(g));
            },
            1 if !out.ops.is_empty() => {
                let at = g.index(out.ops.len());
                out.ops[at] = op(g);
            },
            2 if !out.ops.is_empty() => {
                let at = g.index(out.ops.len());
                out.ops.remove(at);
            },
            _ => out.init = init(g),
        }
        out
    }
}

/// Numbers about a search-params case: init length, operations, and
/// indicators of the shapes it aims at.
pub fn params_metrics(case: &ParamsCase, _size: usize) -> Vec<(&'static str, f64)> {
    let b = |x: bool| f64::from(u8::from(x));
    vec![
        ("init chars", case.init.chars().count() as f64),
        ("operations", case.ops.len() as f64),
        ("sorts", case.ops.iter().filter(|o| o.0 == Op::Sort).count() as f64),
        (
            "astral",
            b(std::iter::once(&case.init)
                .chain(case.ops.iter().map(|o| &o.1))
                .any(|s| s.chars().any(|c| c > '\u{FFFF}'))),
        ),
        (
            "invalid escapes",
            b(["%FF", "%C3", "%E2%82", "%F0%9F%98", "%ED%A0%80", "%80"]
                .iter()
                .any(|e| case.init.contains(e))),
        ),
    ]
}

/// The generator of search-params cases.
#[ephyra::generator(metrics = params_metrics)]
pub fn params_cases() -> impl Gen<Value = ParamsCase> {
    ParamsCases
}
