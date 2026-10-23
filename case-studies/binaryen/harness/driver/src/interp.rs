// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Oracle `interp`: binaryen's own interpreter (`wasm-opt --fuzz-exec-before`,
//! the evaluator that Precompute, ctor-eval and --fuzz-exec trust) against
//! wasmtime on the same module.
//!
//! Both outputs are brought to one canonical form, then compared line by line.
//! What is masked, and why:
//! - trap kinds and exception payloads (fuzz_opt.py maps every exception to
//!   one string; contract 6 in the dossier): `[trap]`, `[exception thrown]`;
//! - NaN sign/payload (spec NaN propagation; Precompute.cpp:24-27): `nan`;
//! - reference identity / JS prototype (printValue in execution-results.h
//!   prints only null/function/object/i31/externref);
//! - float text: binaryen prints f32 through JS numToString of the double,
//!   wasmtime's side prints Rust's shortest repr, so numbers compare by value
//!   (equal as f64, or equal after rounding both to f32).
//! - i64 log arguments: execution-results.h logs them as two i32s; bynexec
//!   does the same under BYNEXEC_SPLIT_I64.
//! Cases are skipped when either side hit a host limit (binaryen prints
//! `[host limit ...]`, e.g. its 200-deep call limit) or the module uses
//! something only one side can run (cont at the host boundary, an unhandled
//! suspend).

fn v128_canon(lanes: [u32; 4]) -> String {
    // Same masking as bynexec's fmt_v128.
    let v: u128 = lanes.iter().enumerate().fold(0u128, |acc, (i, l)| acc | ((*l as u128) << (32 * i)));
    let is_nan32 = |b: u32| (b & 0x7f80_0000) == 0x7f80_0000 && (b & 0x007f_ffff) != 0;
    let is_nan64 =
        |b: u64| (b & 0x7ff0_0000_0000_0000) == 0x7ff0_0000_0000_0000 && (b & 0x000f_ffff_ffff_ffff) != 0;
    let mut parts = Vec::new();
    for half in 0..2 {
        let h = (v >> (64 * half)) as u64;
        if is_nan64(h) {
            parts.push("nan64".to_string());
            continue;
        }
        for lane in 0..2 {
            let l = (h >> (32 * lane)) as u32;
            parts.push(if is_nan32(l) { "nan32".to_string() } else { format!("{l:08x}") });
        }
    }
    format!("v128({})", parts.join(" "))
}

/// Strip `object(...)` (with nested parens) to `object`.
fn strip_objects(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("object(") {
        out.push_str(&rest[..i]);
        out.push_str("object");
        let mut depth = 0;
        let mut end = rest.len();
        for (j, c) in rest[i + 6..].char_indices() {
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                depth -= 1;
                if depth == 0 {
                    end = i + 6 + j + 1;
                    break;
                }
            }
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

fn canon_value_tokens(s: &str) -> String {
    let s = strip_objects(s);
    let toks: Vec<&str> = s.split(' ').collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let t = toks[i];
        if t == "i32x4" && i + 4 < toks.len() {
            let mut lanes = [0u32; 4];
            let mut ok = true;
            for k in 0..4 {
                let raw = toks[i + 1 + k].trim_end_matches(']');
                match u32::from_str_radix(raw.trim_start_matches("0x"), 16) {
                    Ok(x) => lanes[k] = x,
                    Err(_) => ok = false,
                }
            }
            if ok {
                let last = toks[i + 4];
                let tail = if last.ends_with(']') { "]" } else { "" };
                out.push(format!("{}{tail}", v128_canon(lanes)));
                i += 5;
                continue;
            }
        }
        let (core, tail) = match t.strip_suffix(']') {
            Some(c) => (c, "]"),
            None => (t, ""),
        };
        let core = if let Some(r) = core.strip_prefix("i31ref(") {
            format!("i31({r}")
        } else if matches!(core, "nullref" | "nullexternref" | "nullfuncref" | "nullexnref" | "nullcontref") {
            "null".to_string()
        } else if core == "nan" || core == "-nan" || core.starts_with("nan:") || core.starts_with("-nan:") {
            "nan".to_string()
        } else {
            core.to_string()
        };
        out.push(format!("{core}{tail}"));
        i += 1;
    }
    out.join(" ")
}

/// Canonical lines of one output. Binaryen prints the 2nd..nth values of a
/// multivalue result on bare lines; those get the `note result` header.
pub fn canon(out: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut last_note = String::new();
    // An export name may contain newlines, which split a `note result: <name> => `
    // header over several lines; rejoin them so the header is recognised.
    let mut raw: Vec<String> = Vec::new();
    let mut it = out.lines();
    while let Some(l) = it.next() {
        let mut l = l.to_string();
        if l.starts_with("[fuzz-exec] note result: ") && !l.contains(" => ") {
            let mut parts = vec![l.clone()];
            let mut look = it.clone();
            while let Some(n) = look.next() {
                parts.push(n.to_string());
                if n.contains(" => ") {
                    l = parts.join("\n");
                    it = look;
                    break;
                }
            }
        }
        raw.push(l);
    }
    for l in &raw {
        let l = l.trim_end();
        if l.is_empty() {
            continue;
        }
        if l.starts_with("[trap") || l == "[instantiate: trap]" {
            lines.push("[trap]".to_string());
            continue;
        }
        if l.starts_with("[exception thrown") || l == "[instantiate: exception]" {
            lines.push("[exception thrown]".to_string());
            continue;
        }
        if let Some(rest) = l.strip_prefix("[fuzz-exec] note result: ") {
            if let Some(k) = rest.rfind(" => ") {
                last_note = format!("[fuzz-exec] note result: {} => ", &rest[..k]);
                lines.push(format!("{last_note}{}", canon_value_tokens(&rest[k + 4..])));
                continue;
            }
        }
        if !l.starts_with('[') && !last_note.is_empty() {
            lines.push(format!("{last_note}{}", canon_value_tokens(l)));
            continue;
        }
        lines.push(canon_value_tokens(l));
    }
    lines
}

fn num(t: &str) -> Option<f64> {
    let t = t.trim_end_matches(']');
    if t.is_empty() || !(t.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == 'i')) {
        return None;
    }
    t.parse::<f64>().ok()
}

fn tok_eq(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    // bynexec cannot read cont params (wasmtime#10248) and logs every one as
    // `contref`, null or not.
    if a.trim_end_matches(']') == "contref" || b.trim_end_matches(']') == "contref" {
        return true;
    }
    match (num(a), num(b)) {
        (Some(x), Some(y)) => {
            (x == y && x.is_sign_negative() == y.is_sign_negative())
                || ((x as f32) == (y as f32) && (x as f32).is_sign_negative() == (y as f32).is_sign_negative())
        }
        _ => false,
    }
}

fn line_eq(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let ta: Vec<&str> = a.split(' ').collect();
    let tb: Vec<&str> = b.split(' ').collect();
    ta.len() == tb.len() && ta.iter().zip(tb.iter()).all(|(x, y)| tok_eq(x, y))
}

pub enum Verdict {
    Same,
    Skip(&'static str),
    /// (diff class for the signature, first differing lines)
    Differ(&'static str, String),
}

fn class_of(a: &str, b: &str) -> &'static str {
    let both = format!("{a} {b}");
    if a == "[trap]" || b == "[trap]" {
        "trap"
    } else if a == "[exception thrown]" || b == "[exception thrown]" {
        "exception"
    } else if a == "<eof>" || b == "<eof>" {
        "length"
    } else if both.contains("v128(") {
        "simd"
    } else if both.contains("log-branch") {
        "branch"
    } else if ["i31(", "object", "null", "function", "externref", "jserror"].iter().any(|k| both.contains(k)) {
        "ref"
    } else if both.contains('.') || both.contains("inf") || both.contains("nan") {
        "float"
    } else {
        "int"
    }
}

/// Compare binaryen's interpreter output with wasmtime's (both raw).
pub fn compare(binaryen: &str, wasmtime: &str) -> Verdict {
    if binaryen.contains("[host limit") {
        return Verdict::Skip("interp_skip_host_limit");
    }
    if binaryen.contains("unhandled suspend") || wasmtime.contains("[skip: cont at the host boundary]") {
        return Verdict::Skip("interp_skip_cont");
    }
    let a = canon(binaryen);
    let b = canon(wasmtime);
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).map(|s| s.as_str()).unwrap_or("<eof>");
        let y = b.get(i).map(|s| s.as_str()).unwrap_or("<eof>");
        if !line_eq(x, y) {
            return Verdict::Differ(class_of(x, y), format!("line {}: binaryen `{x}` wasmtime `{y}`", i + 1));
        }
    }
    Verdict::Same
}
