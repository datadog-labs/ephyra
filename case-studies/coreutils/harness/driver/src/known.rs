// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Known differences, masked narrowly and counted (stats.json "masked_known").
//! Every rule names the issue that makes it known. Keep this list short: each
//! rule hides every other bug in its class. A rule only fires when the outputs
//! are equal after removing exactly that difference.

use crate::gen::Case;
use crate::Res;

fn has(c: &Case, a: &str) -> bool {
    c.argv().iter().any(|x| x == a)
}

fn words_per_line(b: &[u8]) -> Vec<Vec<&[u8]>> {
    b.split(|&c| c == b'\n').map(|l| l.split(|c| c.is_ascii_whitespace()).filter(|w| !w.is_empty()).collect()).collect()
}

fn strip_line_suffix(b: &[u8], suf: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, l) in b.split(|&c| c == b'\n').enumerate() {
        if i > 0 {
            out.push(b'\n');
        }
        out.extend_from_slice(l.strip_suffix(suf).unwrap_or(l));
    }
    out
}

fn trim_trailing_spaces(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, l) in b.split(|&c| c == b'\n').enumerate() {
        if i > 0 {
            out.push(b'\n');
        }
        let mut e = l.len();
        while e > 0 && l[e - 1] == b' ' {
            e -= 1;
        }
        out.extend_from_slice(&l[..e]);
    }
    out
}

/// `cut -f LIST --complement` where LIST covers every field (e.g. `1-`,
/// `2-,1`), so nothing is selected. GNU 9.12 documents (coreutils.texi
/// 6250-6251, `--only-delimited`): "Normally, any line without a field
/// separator is printed verbatim." So a delimited line prints as an empty line
/// and any other line prints whole. GNU 9.12 does not do that: `printf
/// 'f\na\tb\n' | cut -f1- --complement` prints "\n\n" while `printf 'f\n'`
/// prints "f\n", and `printf f` prints nothing (an unterminated last line is
/// dropped, a delimited one too under `-s`). Returns the documented output
/// when the case is in that shape, so the rule masks only a correct uutils.
fn cut_complement_all_fields_model(c: &Case) -> Option<Vec<u8>> {
    let modes: Vec<&Vec<String>> =
        c.args.iter().filter(|g| matches!(g[0].as_str(), "-b" | "-c" | "-f")).collect();
    if modes.len() != 1 || modes[0][0] != "-f" || modes[0].len() != 2 {
        return None;
    }
    if !has(c, "--complement") {
        return None;
    }
    // coreutils.texi 6250: "For -f, do not print lines that do not contain
    // field separators."
    let only_delimited = has(c, "-s");
    // Union of the ranges must be [1, inf).
    let mut ranges: Vec<(u64, u64)> = Vec::new();
    for part in modes[0][1].split(',') {
        let (lo, hi) = match part.split_once('-') {
            Some((a, b)) => (
                if a.is_empty() { 1 } else { a.parse().ok()? },
                if b.is_empty() { u64::MAX } else { b.parse().ok()? },
            ),
            None => {
                let n = part.parse().ok()?;
                (n, n)
            }
        };
        if lo == 0 || lo > hi {
            return None;
        }
        ranges.push((lo, hi));
    }
    ranges.sort();
    let mut next = 1u64;
    for (lo, hi) in ranges {
        if lo > next {
            return None;
        }
        next = next.max(hi.saturating_add(1));
    }
    if next != u64::MAX {
        return None;
    }
    let ds: Vec<&Vec<String>> = c.args.iter().filter(|g| g[0] == "-d").collect();
    let delim = match ds.as_slice() {
        [] => b'\t',
        [g] if g.len() == 2 && g[1].len() == 1 => g[1].as_bytes()[0],
        _ => return None,
    };
    let term = if has(c, "-z") { 0u8 } else { b'\n' };
    let ops: Vec<&String> = c.args.iter().filter(|g| g.len() == 1 && (g[0] == "-" || !g[0].starts_with('-'))).map(|g| &g[0]).collect();
    let mut srcs: Vec<&[u8]> = Vec::new();
    if ops.is_empty() {
        srcs.push(&c.stdin);
    }
    for o in ops {
        if o == "-" {
            srcs.push(&c.stdin);
        } else {
            srcs.push(&c.files.iter().find(|(n, _)| n == o)?.1);
        }
    }
    let mut out = Vec::new();
    for src in srcs {
        let mut recs: Vec<&[u8]> = src.split(|&b| b == term).collect();
        if recs.last().is_some_and(|r| r.is_empty()) {
            recs.pop();
        }
        for r in recs {
            if r.contains(&delim) {
                out.push(term);
            } else if !only_delimited {
                out.extend_from_slice(r);
                out.push(term);
            }
        }
    }
    Some(out)
}

/// The non-empty files occur in INPUT in order without overlap, the last one
/// ending it (what is left when some sections are dropped).
fn in_order_pieces(input: &[u8], outs: &[&crate::Entry]) -> bool {
    let pieces: Vec<&[u8]> = outs.iter().map(|e| e.data.as_slice()).filter(|d| !d.is_empty()).collect();
    let Some((last, rest)) = pieces.split_last() else {
        return input.is_empty();
    };
    let Some(head) = input.strip_suffix(*last) else {
        return false;
    };
    let mut pos = 0;
    for d in rest {
        match head[pos..].windows(d.len()).position(|w| w == *d) {
            Some(k) => pos += k + d.len(),
            None => return false,
        }
    }
    true
}

/// The non-empty files occur in INPUT in order without overlap (suppressed
/// lines and %RE% skips may also drop the end of the input).
fn in_order_subpieces(input: &[u8], outs: &[&crate::Entry]) -> bool {
    let mut pos = 0;
    for d in outs.iter().map(|e| e.data.as_slice()).filter(|d| !d.is_empty()) {
        match input[pos..].windows(d.len()).position(|w| w == d) {
            Some(k) => pos += k + d.len(),
            None => return false,
        }
    }
    true
}

/// `csplit FILE /RE/[OFFSET] N` where line N exists. coreutils.texi 3520-3521
/// (csplit, pattern N): "Create an output file containing the input up to but
/// not including line @var{n}"; 3506-3507: "An error occurs if a
/// @var{pattern} argument refers to a nonexistent line of the input file".
/// GNU 9.12 also fails with "'N': line number out of range" when line N
/// exists: process_line_count ends with `no_more_lines ()`, which looks past
/// current_line, and after a regex current_line can be ahead of the lines
/// written (the matched line for offsets <= 0, the break line for positive
/// ones). So on 'a\nb\n' `csplit f 2 2` succeeds but `csplit f /b/ 2`, the
/// same split, fails; on 'a\nb\nc\n' `csplit f /c/-1 3` and `/a/+2 3` fail.
/// True only when uutils agrees with every size GNU printed, then prints a
/// non-zero size for the rest after N (so input did remain), and either
/// succeeds with output files that concatenate to the input or fails on a
/// later pattern. Under -s there are no sizes: uutils' files must
/// concatenate to the input (kept ones, under -k, after a later failure).
/// With --suppress-matched (coreutils.texi 3594-3596: "Do not output lines
/// matching the specified @var{pattern}. I.e., suppress the boundary line from
/// the start of the second and subsequent splits"; 3551-3553: the files
/// "contain the original input file, excluding portions skipped with a
/// %@var{regexp}% pattern or the @option{--suppress-matched} option") GNU
/// loses lines instead: on 'a\n2\n' `csplit --suppress-matched f /2/-1 3`
/// writes an empty xx01, prints its size 0 and fails, dropping '2\n', while
/// `/2/-1 /2/` writes '2\n' to xx01, and `/a/ 4` on 'x\na\n2\n' (the same
/// split with regex offset 0) writes '2\n' and succeeds: N may be the line
/// just past the end, since that is the line suppressed.
/// Then GNU's sizes before its last one must be uutils', GNU's last "0", and
/// uutils either fails on a later pattern / repeat as above, or succeeds with
/// files that are in-order pieces of the input, missing at most one line per
/// pattern when there are no %RE% skips or repeats. Under -s with
/// --suppress-matched (no sizes) GNU's kept files must be uutils' first ones.
fn csplit_gnu_stale_line_after_regex(c: &Case, uu: &Res, gnu: &Res) -> bool {
    if gnu.code != Some(1) || crate::contains(&gnu.err, b"on repetition") {
        return false;
    }
    let suppress = has(c, "--suppress-matched");
    let quiet = ["-s", "-q", "--quiet", "--silent"].iter().any(|o| has(c, o));
    // operands (input, then patterns) are the one-element groups that are not options
    let ops: Vec<&str> = c.args.iter().filter(|g| g.len() == 1 && (g[0] == "-" || !g[0].starts_with('-'))).map(|g| g[0].as_str()).collect();
    let Some((&input_name, pats)) = ops.split_first() else {
        return false;
    };
    let is_regex = |p: &str| p.len() > 1 && (p.starts_with('/') || p.starts_with('%'));
    let out_of_range = |p: &str, err: &[u8]| {
        crate::contains(err, format!("'{p}': line number out of range").as_bytes())
            || crate::contains(err, format!("\u{2018}{p}\u{2019}: line number out of range").as_bytes())
    };
    let Some(i) = (0..pats.len()).find(|&i| {
        pats[i].parse::<usize>().is_ok() && pats[..i].iter().any(|p| is_regex(p)) && out_of_range(pats[i], &gnu.err)
    }) else {
        return false;
    };
    let n: usize = pats[i].parse().unwrap();
    let input: &[u8] = if input_name == "-" {
        &c.stdin
    } else {
        match c.files.iter().find(|(f, _)| f == input_name) {
            Some((_, d)) => d,
            None => return false,
        }
    };
    let lines = input.split(|&b| b == b'\n').count() - usize::from(input.ends_with(b"\n") || input.is_empty());
    if n == 0 || n > lines + usize::from(suppress) {
        return false;
    }
    let ul: Vec<&[u8]> = uu.out.split_inclusive(|&b| b == b'\n').collect();
    let gl: Vec<&[u8]> = gnu.out.split_inclusive(|&b| b == b'\n').collect();
    if quiet {
        if !uu.out.is_empty() || !gnu.out.is_empty() {
            return false;
        }
    } else if suppress {
        // GNU printed the size of the (emptied) file for N, then failed
        if gl.last() != Some(&&b"0\n"[..]) || gl.len() > ul.len() || ul[..gl.len() - 1] != gl[..gl.len() - 1] {
            return false;
        }
    } else if gl.len() >= ul.len() || ul[..gl.len()] != gl[..] || ul[gl.len()..].iter().all(|l| *l == b"0\n") {
        return false;
    }
    let mut outs: Vec<&crate::Entry> =
        uu.tree.iter().filter(|e| e.kind == 'f' && !c.files.iter().any(|(f, _)| *f == e.path)).collect();
    outs.sort_by(|a, b| a.path.cmp(&b.path));
    let cat: Vec<u8> = outs.iter().flat_map(|e| e.data.iter().copied()).collect();
    // %RE% sections write no file: then the files are pieces of the
    // input, in order, the last one ending it
    let covers_input = || if pats.iter().any(|p| p.starts_with('%')) { in_order_pieces(input, &outs) } else { cat == input };
    match uu.code {
        // uutils' files are in-order pieces of the input, missing at most one
        // line per pattern (when no %RE% skips and no repeats), none for an N
        // just past the end
        Some(0) if suppress => {
            let out_lines: usize = outs
                .iter()
                .map(|e| e.data.iter().filter(|&&b| b == b'\n').count() + usize::from(!e.data.is_empty() && !e.data.ends_with(b"\n")))
                .sum();
            let counted = !pats.iter().any(|p| p.starts_with('%') || p.starts_with('{'));
            (quiet || outs.len() == ul.len()) && in_order_subpieces(input, &outs) && (!counted || out_lines + pats.len() >= lines + usize::from(n > lines))
        }
        Some(0) => (quiet || outs.len() == ul.len()) && covers_input(),
        // a later pattern, or a repeat of N, is what uutils rejects
        Some(1) => {
            let later = pats[i + 1..].iter().any(|p| {
                out_of_range(p, &uu.err) || crate::contains(&uu.err, format!("'{p}': match not found").as_bytes())
            }) || (pats.get(i + 1).is_some_and(|p| p.starts_with('{'))
                && crate::contains(&uu.err, format!("'{n}': line number out of range on repetition").as_bytes()));
            // under -s there are no sizes: with -k the kept files must still
            // be the input's pieces, and more of them than GNU kept; with
            // --suppress-matched (pieces may miss lines) GNU's kept files
            // must also be uutils' first ones
            let mut gnu_outs: Vec<&crate::Entry> =
                gnu.tree.iter().filter(|e| e.kind == 'f' && !c.files.iter().any(|(f, _)| *f == e.path)).collect();
            gnu_outs.sort_by(|a, b| a.path.cmp(&b.path));
            let pieces = if suppress {
                gnu_outs.iter().zip(&outs).all(|(g, u)| g.path == u.path && g.data == u.data) && in_order_subpieces(input, &outs)
            } else {
                covers_input()
            };
            later && (!quiet || outs.is_empty() || (outs.len() > gnu_outs.len() && pieces))
        }
        _ => false,
    }
}

/// Number of `%q` / `%N$q` directives in a printf format (`%%` is a literal).
pub fn printf_q_directives(f: &str) -> usize {
    let b = f.as_bytes();
    let (mut i, mut n) = (0, 0);
    while i < b.len() {
        if b[i] != b'%' {
            i += 1;
            continue;
        }
        if b.get(i + 1) == Some(&b'%') {
            i += 2;
            continue;
        }
        let mut j = i + 1;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > i + 1 && b.get(j) == Some(&b'$') {
            j += 1;
        } else {
            j = i + 1;
        }
        if b.get(j) == Some(&b'q') {
            n += 1;
        }
        i += 1;
    }
    n
}

/// `printf %q` with no argument left. coreutils.texi 13036-13038: "Missing
/// @var{argument}s are treated as null strings or as zeros, depending on
/// whether the context expects a string or a number", and `printf %q ''`
/// prints `''`. GNU 9.12 prints nothing instead (src/printf.c 578-586 skips
/// the fputs when no argument is left); uutils prints `''`. True when GNU's
/// output is uutils' with between 1 and (number of %q directives) `''` removed:
/// the missing arguments are only those of the last pass over the format.
fn printf_gnu_q_missing_arg(c: &Case, uu: &Res, gnu: &Res) -> bool {
    let Some(f) = c.argv().into_iter().next() else { return false };
    let k = printf_q_directives(&f);
    let (u, g) = (&uu.out, &gnu.out);
    if k == 0 || u.len() > 4096 || u.len() < g.len() + 2 {
        return false;
    }
    // best[j] after processing u[..i]: fewest '' deletions matching g[..j]
    let inf = usize::MAX;
    let mut best: Vec<Vec<usize>> = vec![vec![inf; g.len() + 1]; u.len() + 1];
    best[0][0] = 0;
    for i in 0..u.len() {
        for j in 0..=g.len() {
            let d = best[i][j];
            if d == inf {
                continue;
            }
            if j < g.len() && u[i] == g[j] && d < best[i + 1][j + 1] {
                best[i + 1][j + 1] = d;
            }
            if u[i..].starts_with(b"''") && d + 1 < best[i + 2][j] {
                best[i + 2][j] = d + 1;
            }
        }
    }
    let d = best[u.len()][g.len()];
    d >= 1 && d <= k
}

/// Split into maximal runs of decimal-number bytes (digits, '.', '-') and
/// the bytes between them; the bool says whether the run is numeric.
fn numeric_runs(b: &[u8]) -> Vec<(bool, &[u8])> {
    let isnum = |c: u8| c.is_ascii_digit() || c == b'.' || c == b'-';
    let mut out: Vec<(bool, &[u8])> = Vec::new();
    let mut start = 0;
    for i in 1..=b.len() {
        if i == b.len() || isnum(b[i]) != isnum(b[start]) {
            if start < i {
                out.push((isnum(b[start]), &b[start..i]));
            }
            start = i;
        }
    }
    out
}

/// `-?D*.F+` as (value scaled by 10^len(F), len(F)).
fn fixed_decimal(t: &[u8]) -> Option<(i128, usize)> {
    let t = std::str::from_utf8(t).ok()?;
    let (neg, t) = t.strip_prefix('-').map_or((false, t), |r| (true, r));
    let (i, f) = t.split_once('.')?;
    if f.is_empty() || f.len() > 18 || i.len() > 18 || !(i.bytes().chain(f.bytes()).all(|c| c.is_ascii_digit())) {
        return None;
    }
    let v: i128 = format!("{i}{f}").parse().ok()?;
    Some((if neg { -v } else { v }, f.len()))
}

/// Decimal number text normalised for an exact numeric comparison.
fn norm_decimal(t: &str) -> Option<String> {
    let (neg, t) = t.strip_prefix('-').map_or((false, t), |r| (true, r));
    let (i, f) = t.split_once('.').unwrap_or((t, ""));
    if i.is_empty() && f.is_empty() || !i.bytes().chain(f.bytes()).all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (i, f) = (i.trim_start_matches('0'), f.trim_end_matches('0'));
    let neg = neg && !(i.is_empty() && f.is_empty());
    Some(format!("{}{i}.{f}", if neg { "-" } else { "" }))
}

/// A normalised decimal (see norm_decimal) times `from` / `to`, exactly, normalised
/// again. `to` is 1, 10, 1000 or 1024, so the result is a terminating decimal:
/// dividing by 1024 is multiplying by 5^10 / 10^10.
fn scale_decimal(v: &str, from: i128, to: i128) -> Option<String> {
    let (neg, t) = v.strip_prefix('-').map_or((false, v), |r| (true, r));
    let (i, f) = t.split_once('.')?;
    let mut m: i128 = format!("{i}{f}").parse().ok()?;
    let mut decimals = f.len();
    m = m.checked_mul(from)?;
    match to {
        1 => {}
        10 => decimals += 1,
        1000 => decimals += 3,
        1024 => {
            m = m.checked_mul(5_i128.pow(10))?;
            decimals += 10;
        }
        _ => return None,
    }
    let digits = format!("{m:0>width$}", width = decimals + 1);
    let (ip, fp) = digits.split_at(digits.len() - decimals);
    norm_decimal(&format!("{}{ip}.{fp}", if neg { "-" } else { "" }))
}

/// GNU 9.12 numfmt rounds in `long double` (x87, 64-bit mantissa): an input
/// like `0.54` is stored a little above 0.54, `0.54 * 100` comes out above 54,
/// and the default `--round=from-zero` (also `up`) then prints `0.55`
/// (`numfmt 0.54` -> 0.55, `numfmt 1.54` -> 1.54). Printing an exact input
/// value with its own number of decimals needs no rounding at all, so GNU's
/// output is a wrong result; uutils prints the input value. Masked only
/// without suffix scaling (no --to other than none), with the same exit status
/// and stderr either empty on both sides or non-empty on both (the oracle only
/// compares that much: an error line elsewhere in the input, e.g. uutils'
/// "rejecting suffix in input: '2K'" for GNU's '2k', must not unmask), when stdout is equal except for numbers where GNU is exactly one
/// unit in the last decimal place away from uutils and uutils' number is,
/// exactly, a number of the input. With --from-unit/--to-unit of 1, 10, 1000 or
/// 1024 (the last of each counts) the input numbers are first scaled exactly:
/// `numfmt --from-unit=10 --round=up 1.856` is 18.560, GNU prints 18.561;
/// `numfmt --from-unit=1024 --to-unit=10 --format=%.2f 3` is 307.20, GNU 307.21.
fn numfmt_gnu_long_double_last_digit(c: &Case, uu: &Res, gnu: &Res) -> bool {
    let argv = c.argv();
    let (mut from, mut to) = (1i128, 1i128);
    for a in &argv {
        if a.starts_with("--to=") && a != "--to=none" {
            return false;
        }
        for (pre, unit) in [("--from-unit=", &mut from), ("--to-unit=", &mut to)] {
            if let Some(u) = a.strip_prefix(pre) {
                *unit = match u {
                    "1" | "10" | "1000" | "1024" => u.parse().unwrap(),
                    _ => return false,
                };
            }
        }
    }
    if uu.code != gnu.code || uu.err.is_empty() != gnu.err.is_empty() || uu.out == gnu.out {
        return false;
    }
    let mut inputs: Vec<String> = Vec::new();
    for src in argv.iter().map(|a| a.as_bytes()).chain(std::iter::once(&c.stdin[..])) {
        for (n, t) in numeric_runs(src) {
            if let Some(v) = n.then(|| std::str::from_utf8(t).ok()).flatten().and_then(norm_decimal) {
                inputs.extend(scale_decimal(&v, from, to));
            }
        }
    }
    let (ru, rg) = (numeric_runs(&uu.out), numeric_runs(&gnu.out));
    if ru.len() != rg.len() {
        return false;
    }
    let mut differed = false;
    for ((nu, tu), (ng, tg)) in ru.iter().zip(&rg) {
        if nu != ng {
            return false;
        }
        if tu == tg {
            continue;
        }
        let (Some((vu, pu)), Some((vg, pg))) = (fixed_decimal(tu), fixed_decimal(tg)) else { return false };
        let in_input = std::str::from_utf8(tu).ok().and_then(norm_decimal).is_some_and(|v| inputs.contains(&v));
        if !*nu || pu != pg || (vu - vg).abs() != 1 || !in_input {
            return false;
        }
        differed = true;
    }
    differed
}

/// `printf` where a numeric argument fails to convert and a `\c` escape (in
/// FORMAT or a `%b` argument) then stops output. coreutils.info (printf
/// invocation): "An exit status of zero indicates success, and a nonzero
/// value indicates failure"; POSIX printf: a diagnostic is written and "the
/// utility shall not exit with a zero exit status". GNU 9.12 (black box):
/// `printf '%d' x` prints "'x': expected a numeric value" and exits 1, but
/// `printf '%d\c' x` and `printf '%d%b' x '\c'` print the same diagnostic
/// and exit 0; uutils exits 1 in all of them. True only when the outputs and
/// files are equal, GNU exits 0 and uutils 1, an argument holds `\c`, and
/// every stderr line of both is a numeric-conversion diagnostic (the same
/// number of them).
fn printf_gnu_cancel_drops_failure(c: &Case, uu: &Res, gnu: &Res) -> bool {
    const DIAG: [&[u8]; 3] = [b": expected a numeric value", b": value not completely converted", b": Numerical result out of range"];
    let diags = |err: &[u8]| -> Option<usize> {
        let lines: Vec<&[u8]> = err.split(|&b| b == b'\n').filter(|l| !l.is_empty()).collect();
        (!lines.is_empty() && lines.iter().all(|l| l.starts_with(b"printf: ") && DIAG.iter().any(|d| l.ends_with(d))))
            .then_some(lines.len())
    };
    gnu.code == Some(0)
        && uu.code == Some(1)
        // stdout may also differ by GNU's dropped `''` for a %q with no
        // argument left (see printf_gnu_q_missing_arg), a separate deviation.
        && (uu.out == gnu.out || printf_gnu_q_missing_arg(c, uu, gnu))
        && uu.tree == gnu.tree
        && c.argv().iter().any(|a| a.contains("\\c"))
        && diags(&gnu.err).is_some_and(|n| diags(&uu.err) == Some(n))
}

/// `split -n l/N` where N exceeds the input size. coreutils.info (split
/// invocation, -n): "l/N generate N files without splitting lines or
/// records" and "All N files are created even if there are fewer than N
/// lines". GNU 9.12 (black box): when the input ends with its record
/// separator and is shorter than N bytes ('x\n' with l/3, 'abcd\n' with
/// l/6) it keeps creating files until the suffixes run out and fails with
/// "output file suffixes exhausted" (676 files, 100 with -d); uutils writes
/// N files and exits 0. True only for an l/N (not l/K/N) chunk spec, N
/// larger than the input, uutils exit 0 with empty stderr, and GNU exit 1
/// with only that diagnostic.
fn split_gnu_line_chunks_exhaust_suffixes(c: &Case, uu: &Res, gnu: &Res) -> bool {
    let argv = c.argv();
    let mut spec = None;
    for (i, a) in argv.iter().enumerate() {
        if a == "-n" || (a.len() >= 6 && "--number".starts_with(a.as_str())) {
            spec = argv.get(i + 1).cloned();
        } else if let Some(v) = a.strip_prefix("--number=").or_else(|| a.strip_prefix("-n")) {
            spec = Some(v.to_string());
        }
    }
    let Some(n) = spec.as_deref().and_then(|s| s.strip_prefix("l/")).and_then(|n| n.parse::<u64>().ok()) else {
        return false;
    };
    // The input is the first operand (f1 or "-"), stdin when there is none.
    let input = if argv.iter().any(|a| a == "-") {
        &c.stdin
    } else {
        c.files.iter().find(|(f, _)| argv.contains(f)).map_or(&c.stdin, |(_, data)| data)
    };
    (input.len() as u64) < n
        && uu.code == Some(0)
        && uu.err.is_empty()
        && gnu.code == Some(1)
        && gnu.err == b"split: output file suffixes exhausted\n"
}

pub fn known(c: &Case, uu: &Res, gnu: &Res) -> Option<&'static str> {
    let same_status = uu.code == gnu.code && uu.tree == gnu.tree;
    match c.util {
        // #15254 `wc -l -c`: GNU sizes the columns from the input's size when
        // the input is a regular file; uutils uses width 7. Only column widths.
        "wc" if same_status && uu.out != gnu.out && words_per_line(&uu.out) == words_per_line(&gnu.out) => {
            Some("wc_column_width")
        }
        // #15254 `cksum -`: GNU prints the name "-" for an explicit "-" operand.
        "cksum" | "sum" | "md5sum" | "sha1sum" | "sha256sum" | "sha512sum" | "b2sum"
            if same_status && has(c, "-") && strip_line_suffix(&uu.out, b" -") == strip_line_suffix(&gnu.out, b" -") =>
        {
            Some("cksum_dash_name")
        }
        // #15254 `basenc --base64 -d`: on invalid input GNU writes the decoded
        // prefix before failing, uutils writes nothing. Both must fail.
        "base64" | "base32" | "basenc"
            if uu.code == gnu.code && gnu.code != Some(0) && (has(c, "-d") || has(c, "--decode")) && gnu.out.starts_with(&uu.out) =>
        {
            Some("basenc_decode_error_prefix")
        }
        // #15254 `join -a 1`: uutils leaves a trailing separator on unpaired lines.
        "join" if same_status && trim_trailing_spaces(&uu.out) == trim_trailing_spaces(&gnu.out) && uu.out != gnu.out => {
            Some("join_unpaired_trailing_space")
        }
        // #14705 / PR #14717: `split -u`/`--unbuffered` is not implemented ("it reports
        // options we do not implement at all: ... `split -u`/`--unbuffered` ... Of little
        // value for us, so it is being skipped"). Only clap's rejection of the option.
        "split"
            if uu.code == Some(1)
                && crate::contains(&uu.err, b"unexpected argument '-")
                && c.argv().iter().any(|a| (a.starts_with("-u") && !a.starts_with("--")) || (a.len() >= 4 && "--unbuffered".starts_with(a.as_str()))) =>
        {
            Some("split_unbuffered_unimplemented")
        }
        // `split --numeric-suffixes=FROM -n N` with FROM >= N whose last suffix does not fit
        // the fixed width: uutils documents and implements an up-front check (filenames.rs
        // Suffix::from: "Do NOT pre-calculate new suffix length otherwise, i.e. when suffix
        // start >= N number of files") and fails before writing; GNU writes the files that fit
        // and then fails with "output file suffixes exhausted". Both exit 1; coreutils.texi
        // only says "you may also want to use option -a to allow suffixes beyond 99".
        "split"
            if uu.code == Some(1)
                && gnu.code == Some(1)
                && crate::contains(&uu.err, b"the suffix length needs to be at least")
                && gnu.err.ends_with(b"output file suffixes exhausted\n")
                && c.argv().iter().any(|a| a.starts_with("--numeric-suffixes=") || a.starts_with("--hex-suffixes=")) =>
        {
            Some("split_number_suffix_start_overflow_checked_early")
        }
        // GNU 9.12 runs out of suffixes on `-n l/N` with N above the input size
        // (see split_gnu_line_chunks_exhaust_suffixes).
        "split" if split_gnu_line_chunks_exhaust_suffixes(c, uu, gnu) => Some("split_gnu_line_chunks_exhaust_suffixes"),
        // #15254 `fmt -g 1`: goal width handling differs.
        "fmt" if c.argv().iter().any(|a| a == "-g") && uu.out != gnu.out => Some("fmt_goal"),
        // #15254 (`sort --month-sort -k 2 --hu`, ...): uutils accepts ordering
        // options that GNU rejects as incompatible.
        "sort" if gnu.code == Some(2) && crate::contains(&gnu.err, b"incompatible") => Some("sort_incompatible_opts"),
        // #15254 `unexpand --first-only -`: clap rejects "-" after --first-only.
        "unexpand" if has(c, "--first-only") && has(c, "-") && uu.code != gnu.code => Some("unexpand_first_only_dash"),
        // #15254 `od --strings=2 -o`: uutils accepts -S together with an output type.
        "od" if uu.code == Some(0) && crate::contains(&gnu.err, b"no type may be specified when dumping strings") => {
            Some("od_strings_with_type")
        }
        // #15254 `fold --bytes --ch`: clap makes -b and -c conflict; GNU accepts both.
        "fold" if has(c, "-b") && has(c, "-c") && uu.code != gnu.code => Some("fold_bytes_and_characters"),
        // GNU 9.12 contradicts its own manual for `cut -f 1- --complement`
        // (see cut_complement_all_fields_model). Masked only when uutils
        // prints exactly what the manual describes.
        "cut" if same_status
            && uu.code == Some(0)
            && uu.out != gnu.out
            && cut_complement_all_fields_model(c).is_some_and(|m| m == uu.out) =>
        {
            Some("cut_complement_all_fields_gnu_undelimited")
        }
        // GNU 9.12 rejects an existing line number after a regex pattern
        // (see csplit_gnu_stale_line_after_regex).
        "csplit" if csplit_gnu_stale_line_after_regex(c, uu, gnu) => Some("csplit_gnu_line_out_of_range_after_regex"),
        // GNU 9.12 prints nothing for %q with a missing argument, against
        // its manual (see printf_gnu_q_missing_arg).
        "printf" if same_status && uu.err.is_empty() == gnu.err.is_empty() && printf_gnu_q_missing_arg(c, uu, gnu) => {
            Some("printf_gnu_q_missing_argument")
        }
        // GNU 9.12 `\c` exits 0 after a numeric conversion error
        // (see printf_gnu_cancel_drops_failure).
        "printf" if printf_gnu_cancel_drops_failure(c, uu, gnu) => Some("printf_gnu_cancel_exit_success"),
        // GNU 9.12 rounds an exact input decimal off by one in the last
        // place (see numfmt_gnu_long_double_last_digit).
        "numfmt" if same_status && numfmt_gnu_long_double_last_digit(c, uu, gnu) => {
            Some("numfmt_gnu_long_double_last_digit")
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn res(out: &str) -> Res {
        Res { code: Some(0), signal: None, timeout: false, out: out.into(), err: vec![], tree: vec![] }
    }

    fn case(args: &[&str], stdin: &str) -> Case {
        Case { util: "numfmt", args: args.iter().map(|a| vec![a.to_string()]).collect(), stdin: stdin.into(), files: vec![] }
    }

    #[test]
    fn csplit_suppress_matched_stale_line_mask() {
        let ent = |p: &str, d: &str| crate::Entry { path: p.into(), kind: 'f', mode: 0o644, data: d.into() };
        let c = |n: &str| Case {
            util: "csplit",
            args: ["--suppress-matched", "f1", "/2/-1", n].iter().map(|a| vec![a.to_string()]).collect(),
            stdin: vec![],
            files: vec![("f1".into(), b"a\n2\n".to_vec())],
        };
        let mut gnu = res("0\n0\n");
        gnu.code = Some(1);
        gnu.err = "csplit: '3': line number out of range\n".into();
        let mut uu = res("0\n2\n0\n");
        uu.tree = vec![ent("f1", "a\n2\n"), ent("xx00", ""), ent("xx01", "2\n"), ent("xx02", "")];
        assert!(csplit_gnu_stale_line_after_regex(&c("3"), &uu, &gnu));
        // uutils losing the line too is not GNU's bug
        let mut lost = res("0\n0\n0\n");
        lost.tree = vec![ent("f1", "a\n2\n"), ent("xx00", ""), ent("xx01", ""), ent("xx02", "")];
        assert!(!csplit_gnu_stale_line_after_regex(&c("3"), &lost, &gnu));
        // past the line after the end: a real out of range
        gnu.err = "csplit: '4': line number out of range\n".into();
        assert!(!csplit_gnu_stale_line_after_regex(&c("4"), &uu, &gnu));
    }

    #[test]
    fn csplit_quiet_keep_stale_line_mask() {
        // cluster diff-csplit-files-line-after-regex: f1='\n', `csplit -s -k f1 /^$/ 1 3`
        let ent = |p: &str, d: &str| crate::Entry { path: p.into(), kind: 'f', mode: 0o644, data: d.into() };
        let c = |n: &str| Case {
            util: "csplit",
            args: ["-s", "-k", "f1", "/^$/", "1", n].iter().map(|a| vec![a.to_string()]).collect(),
            stdin: vec![],
            files: vec![("f1".into(), b"\n".to_vec())],
        };
        let mut gnu = res("");
        gnu.code = Some(1);
        gnu.err = "csplit: '1': line number out of range\n".into();
        gnu.tree = vec![ent("f1", "\n"), ent("xx00", ""), ent("xx01", "")];
        let mut uu = res("");
        uu.code = Some(1);
        uu.err = "csplit: '3': line number out of range\n".into();
        uu.tree = vec![ent("f1", "\n"), ent("xx00", ""), ent("xx01", ""), ent("xx02", "\n")];
        assert!(csplit_gnu_stale_line_after_regex(&c("3"), &uu, &gnu));
        // uutils' kept files losing the line is not GNU's bug
        uu.tree[3] = ent("xx02", "");
        assert!(!csplit_gnu_stale_line_after_regex(&c("3"), &uu, &gnu));
    }

    #[test]
    fn csplit_quiet_suppress_matched_stale_line_mask() {
        // cluster diff-csplit-files-suppress-matched-line-after-regex:
        // f1='\n\n\n1\n', `csplit -k --suppress-matched -s f1 '/[0-9]/-1' 3 6`
        let ent = |p: &str, d: &str| crate::Entry { path: p.into(), kind: 'f', mode: 0o644, data: d.into() };
        let c = Case {
            util: "csplit",
            args: ["-k", "--suppress-matched", "-s", "f1", "/[0-9]/-1", "3", "6"].iter().map(|a| vec![a.to_string()]).collect(),
            stdin: vec![],
            files: vec![("f1".into(), b"\n\n\n1\n".to_vec())],
        };
        let mut gnu = res("");
        gnu.code = Some(1);
        gnu.err = "csplit: '3': line number out of range\n".into();
        gnu.tree = vec![ent("f1", "\n\n\n1\n"), ent("xx00", "\n\n"), ent("xx01", "")];
        let mut uu = res("");
        uu.code = Some(1);
        uu.err = "csplit: '6': line number out of range\n".into();
        uu.tree = vec![ent("f1", "\n\n\n1\n"), ent("xx00", "\n\n"), ent("xx01", ""), ent("xx02", "")];
        assert!(csplit_gnu_stale_line_after_regex(&c, &uu, &gnu));
        // uutils' first file differing from GNU's is not GNU's bug
        uu.tree[1] = ent("xx00", "\n");
        assert!(!csplit_gnu_stale_line_after_regex(&c, &uu, &gnu));
        // nor files that are not pieces of the input
        uu.tree[1] = ent("xx00", "\n\n");
        uu.tree[3] = ent("xx02", "x\n");
        assert!(!csplit_gnu_stale_line_after_regex(&c, &uu, &gnu));
    }

    #[test]
    fn numfmt_long_double_mask() {
        let m = |c: &Case, u: &str, g: &str| numfmt_gnu_long_double_last_digit(c, &res(u), &res(g));
        assert!(m(&case(&["--field=2-"], "k .54"), "k 0.54", "k 0.55"));
        assert!(m(&case(&["--", "0.54"], ""), "0.54\n", "0.55\n"));
        // uutils' number is not an input number: a rounding of its own
        assert!(!m(&case(&["--format=%.1f"], "0.25"), "0.2", "0.3"));
        // scaling: off-by-one there is a real difference
        assert!(!m(&case(&["--to=si"], "12.1"), "12", "13"));
        // exact unit scaling by a power of ten: uutils' number is the input moved
        assert!(m(&case(&["--from-unit=10", "--round=up"], "1.856"), "18.560", "18.561"));
        assert!(m(&case(&["--to-unit=10"], "-99.40"), "-9.94", "-9.95"));
        assert!(m(&case(&["--to-unit=1000", "--from-unit=10"], "0.29"), "0.0029", "0.0028"));
        assert!(!m(&case(&["--to-unit=10"], "5.46"), "0.55", "0.54"));
        assert!(!m(&case(&["--to-unit=1024"], "5.4"), "0.01", "0.02"));
        assert!(m(&case(&["--from-unit=1024", "--to-unit=10", "--format=%08.2f"], "3"), "00307.20", "00307.21"));
        assert!(m(&case(&["--to-unit=1024"], "1.0"), "0.0009765625", "0.0009765626"));
        assert!(!m(&case(&["--to-unit=1024", "--format=%.2f"], "3"), "0.01", "0.00"));
        assert!(!m(&case(&["--from-unit=10"], "1.856"), "1.856", "1.857"));
        // search finding: GNU 8.7653 for an exact 8.7654 that needs no rounding
        assert!(m(&case(&["--round=down", "8.7654 1.25"], ""), "8.7654 1.25\n", "8.7653 1.25\n"));
        // search finding: a later line fails on both sides, stderr wording differs
        let e = |out: &str, err: &str| Res { code: Some(2), err: err.into(), ..res(out) };
        let c = case(&["--round=towards-zero"], "8.7654\n2k");
        let (eu, eg) = ("numfmt: rejecting suffix in input: '2K' (consider using --from)\n", "numfmt: rejecting suffix in input: '2k' (consider using --from)\n");
        assert!(numfmt_gnu_long_double_last_digit(&c, &e("8.7654\n", eu), &e("8.7653\n", eg)));
        assert!(!numfmt_gnu_long_double_last_digit(&c, &e("8.7654\n", ""), &e("8.7653\n", eg)));
        // search finding: --from-unit=1000 moves 8.7654 to 8765.4, exact at %.1f
        assert!(m(&case(&["--format=%.1f", "--from-unit=1000", "--round=towards-zero", "8.7654"], ""), "8765.4\n", "8765.3\n"));
        assert!(m(&case(&["--from-unit=1000", "--round=down", "8.7654"], ""), "8765.4000\n", "8765.3999\n"));
        // more than one unit apart, or other text differing
        assert!(!m(&case(&[], "0.54"), "0.54", "0.56"));
        assert!(!m(&case(&[], "0.54 x"), "0.54 x", "0.55 y"));
        assert!(!m(&case(&[], "0.54"), "0.54", "0.54"));
    }

    #[test]
    fn printf_cancel_exit_mask() {
        let pc = |a: &[&str]| Case { util: "printf", ..case(a, "") };
        let r = |code: i32, err: &str| Res { code: Some(code), err: err.into(), ..res("0") };
        let e = "printf: 'x': expected a numeric value\n";
        assert!(printf_gnu_cancel_drops_failure(&pc(&["%d\\c", "x"]), &r(1, e), &r(0, e)));
        assert!(printf_gnu_cancel_drops_failure(&pc(&["%d%b", "x", "\\c"]), &r(1, e), &r(0, e)));
        // no \c: GNU exiting 0 is a real difference
        assert!(!printf_gnu_cancel_drops_failure(&pc(&["%d", "x"]), &r(1, e), &r(0, e)));
        // other diagnostics, or the reverse direction, stay visible
        assert!(!printf_gnu_cancel_drops_failure(&pc(&["%d\\c", "x"]), &r(1, "printf: invalid precision\n"), &r(0, "printf: invalid precision\n")));
        assert!(!printf_gnu_cancel_drops_failure(&pc(&["%d\\c", "x"]), &r(0, e), &r(1, e)));
        assert!(!printf_gnu_cancel_drops_failure(&pc(&["%d\\c", "x"]), &r(1, e), &r(0, "")));
        // the triaged cluster's examples (uutils and GNU quote the operand differently)
        let ex = |a: &[&str], out: &str, ue: &str, ge: &str| {
            let r = |code: i32, err: &str| Res { code: Some(code), err: err.into(), ..res(out) };
            printf_gnu_cancel_drops_failure(&pc(a), &r(1, ue), &r(0, ge))
        };
        assert!(ex(&["%q%*.*E%.5d\\c", "\\0101", "3 "], "'\\0101'0E+0000000",
            "printf: '3\\ ': value not completely converted\n", "printf: '3 ': value not completely converted\n"));
        assert!(ex(&["%2e%-.*s\\c% q", "-2", "0.125"], "-2.000000e+00",
            "printf: '0.125': value not completely converted\n", "printf: '0.125': value not completely converted\n"));
        assert!(ex(&["%07E\\c%q%008.0o", "\\c"], "0.000000E+00",
            "printf: '\\\\c': expected a numeric value\n", "printf: '\\\\c': expected a numeric value\n"));
        // combined with GNU dropping '' for a %q with no argument left
        let e2 = "printf: '\\\\c': expected a numeric value\n";
        let a = pc(&["%*.*G%q%0.5u\\c", "\\c"]);
        let ro = |code: i32, out: &str| Res { code: Some(code), err: e2.into(), ..res(out) };
        assert!(printf_gnu_cancel_drops_failure(&a, &ro(1, "0''00000"), &ro(0, "000000")));
        // any other stdout difference stays visible
        assert!(!printf_gnu_cancel_drops_failure(&a, &ro(1, "0''00001"), &ro(0, "000000")));
        assert!(!printf_gnu_cancel_drops_failure(&pc(&["%d\\c", "x"]), &ro(1, "0''"), &ro(0, "0")));
    }

    #[test]
    fn split_line_chunks_exhaust_mask() {
        let sc = |args: &[&str], stdin: &str| Case {
            util: "split",
            args: args.iter().map(|a| vec![a.to_string()]).collect(),
            stdin: stdin.into(),
            files: vec![("f1".into(), b"abcdefgh\n".to_vec())],
        };
        let r = |code: i32, err: &str| Res { code: Some(code), signal: None, timeout: false, out: vec![], err: err.into(), tree: vec![] };
        let e = "split: output file suffixes exhausted\n";
        assert!(split_gnu_line_chunks_exhaust_suffixes(&sc(&["-n", "l/3", "-a", "0", "-d"], "x\n"), &r(0, ""), &r(1, e)));
        assert!(split_gnu_line_chunks_exhaust_suffixes(&sc(&["--number=l/5", "-"], "ab\n"), &r(0, ""), &r(1, e)));
        // N not above the input size, K/N form, other modes, other errors
        assert!(!split_gnu_line_chunks_exhaust_suffixes(&sc(&["-n", "l/2"], "x\n"), &r(0, ""), &r(1, e)));
        assert!(!split_gnu_line_chunks_exhaust_suffixes(&sc(&["-n", "l/5", "f1"], "x\n"), &r(0, ""), &r(1, e)));
        assert!(!split_gnu_line_chunks_exhaust_suffixes(&sc(&["-n", "l/1/3"], "x\n"), &r(0, ""), &r(1, e)));
        assert!(!split_gnu_line_chunks_exhaust_suffixes(&sc(&["-n", "3"], "x\n"), &r(0, ""), &r(1, e)));
        assert!(!split_gnu_line_chunks_exhaust_suffixes(&sc(&["-n", "l/3"], "x\n"), &r(1, e), &r(1, e)));
        assert!(!split_gnu_line_chunks_exhaust_suffixes(&sc(&["-n", "l/3"], "x\n"), &r(0, ""), &r(1, "split: invalid\n")));
    }
}
