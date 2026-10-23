// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Case generators: one command line per case, built per utility from the
//! options GNU 9.12 documents (`--help`, the manual), over small value pools
//! so that fields, keys and lines collide. Inputs are ASCII (plus a few control
//! bytes): non-ASCII bytes in the C locale are a known, already-reported class
//! (#9712, #7600), see dossier.

use ephyra::Source;
use std::ops::ControlFlow;

#[derive(Clone, Debug)]
pub struct Case {
    pub util: &'static str,
    /// Argument groups (an option together with its value); flattened to argv.
    pub args: Vec<Vec<String>>,
    pub stdin: Vec<u8>,
    /// Files created in the working directory before the run.
    pub files: Vec<(String, Vec<u8>)>,
    /// The first `keep` argument groups are never dropped by the shrinker: without
    /// them the case compares something that differs by construction (times,
    /// absolute paths, the inherited environment).
    pub keep: usize,
}

impl Case {
    pub fn argv(&self) -> Vec<String> {
        self.args.iter().flatten().cloned().collect()
    }
}

type OptFn = fn(&mut Source) -> Vec<String>;

fn v(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|s| s.to_string()).collect()
}
fn one(s: String) -> Vec<String> {
    vec![s]
}
fn pick<'a>(g: &mut Source, xs: &[&'a str]) -> &'a str {
    xs[g.index(xs.len())]
}
fn num(g: &mut Source, lo: i64, hi: i64) -> String {
    g.int(lo..=hi).to_string()
}

/// 0..=max options, biased towards few, drawn from `table` (repeats allowed).
/// Each option at most once: repeated and conflicting options (clap's "cannot be
/// used multiple times", GNU's "last one wins") are a known bulk class (#15254).
fn opts(g: &mut Source, max: usize, table: &[OptFn]) -> Vec<Vec<String>> {
    let n = if g.bool_with(0.15) { 0 } else { g.int(1..=max) };
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for _ in 0..n {
        let o = table[g.index(table.len())](g);
        let k = opt_key(&o[0]);
        if !seen.contains(&k) {
            seen.push(k);
            out.push(o);
        }
    }
    out
}

fn opt_key(s: &str) -> String {
    let k = if s.starts_with("--") { s.split('=').next().unwrap_or(s) } else { &s[..s.len().min(2)] };
    // options that override each other count as one
    match k {
        "-n" | "-c" | "--lines" | "--bytes" => "count".into(),
        "--check-order" | "--nocheck-order" => "order".into(),
        "--separator" => "-s".into(),
        "--tabs" => "-t".into(),
        "-d" | "-x" | "--numeric-suffixes" | "--hex-suffixes" => "suffixes".into(),
        "--suffix" => "-S".into(),
        "--parents" => "-p".into(),
        "--backup" => "-b".into(),
        "--relative" => "-r".into(),
        "--dir" => "-d".into(),
        _ => k.into(),
    }
}

// ---------------------------------------------------------------- text

const WORDS: &[&str] = &[
    "a", "b", "c", "A", "B", "ab", "ba", "abc", "x", "10", "2", "-3", "1.5", "0", "007", "1e3", "1K",
    "2M", "3Gi", "-0", "+5", "ff", "0x1f", "Jan", "feb", "1.2.3", "v2", "1.10", "-", "", " ", "zz",
];
const DELIMS: &[&str] = &[" ", "\t", ":", ",", "  ", " \t"];
const CTRL: &[u8] = &[0, b'\r', 1, 0x7f, 0x0b, 0x0c, 0x08, 0x1b];

fn line(g: &mut Source, d: &str) -> String {
    let n = g.int(0..=4usize);
    let mut s = String::new();
    if g.bool_with(0.1) {
        s.push_str(pick(g, &[" ", "\t", "  "]));
    }
    for i in 0..n {
        if i > 0 {
            s.push_str(if g.bool_with(0.85) { d } else { pick(g, DELIMS) });
        }
        s.push_str(pick(g, WORDS));
    }
    if g.bool_with(0.08) {
        s.push_str(pick(g, &[" ", "\t"]));
    }
    s
}

fn lines(g: &mut Source, maxn: usize) -> Vec<String> {
    let d = pick(g, DELIMS);
    let n = g.int(0..=maxn);
    let mut out: Vec<String> = Vec::new();
    for _ in 0..n {
        if !out.is_empty() && g.bool_with(0.25) {
            let i = g.index(out.len());
            out.push(out[i].clone()); // duplicates, for uniq/sort -u/comm
        } else {
            out.push(line(g, d));
        }
    }
    out
}

fn finish(g: &mut Source, ls: Vec<String>, ctrl: f64) -> Vec<u8> {
    let mut b = ls.join("\n").into_bytes();
    if !ls.is_empty() && !g.bool_with(0.15) {
        b.push(b'\n');
    }
    if !b.is_empty() && g.bool_with(ctrl) {
        let at = g.index(b.len() + 1);
        b.insert(at, CTRL[g.index(CTRL.len())]);
    }
    b
}

pub fn text(g: &mut Source) -> Vec<u8> {
    let ls = lines(g, 8);
    finish(g, ls, 0.05)
}

fn sorted_text(g: &mut Source) -> Vec<u8> {
    let mut ls = lines(g, 7);
    if g.bool_with(0.9) {
        ls.sort(); // C-locale byte order
    }
    finish(g, ls, 0.0)
}

/// key-first lines for join: "k<d>v<d>v".
fn keyed_text(g: &mut Source, d: &str) -> Vec<u8> {
    let n = g.int(0..=6usize);
    let mut ls: Vec<String> = (0..n)
        .map(|_| {
            let k = pick(g, &["a", "b", "c", "d", "1", "2", "B", ""]);
            let m = g.int(0..=2usize);
            let mut s = k.to_string();
            for _ in 0..m {
                s.push_str(d);
                // empty fields only with an explicit -t: with blank separators
                // they become runs of blanks, which uutils joins differently (found here)
                let vals: &[&str] = if d == " " { &["x", "y", "1", "zz"] } else { &["x", "y", "1", "", "zz"] };
                s.push_str(pick(g, vals));
            }
            s
        })
        .collect();
    if g.bool_with(0.9) {
        ls.sort();
    }
    finish(g, ls, 0.0)
}

fn std_files(g: &mut Source) -> Vec<(String, Vec<u8>)> {
    vec![("f1".into(), text(g)), ("f2".into(), text(g))]
}

/// File operands for filters: mostly stdin, sometimes files, `-`, a missing file.
fn inputs(g: &mut Source) -> Vec<Vec<String>> {
    let c = *g.weighted(&[(6, 0), (3, 1), (1, 2), (2, 3), (1, 4), (1, 5)]);
    match c {
        0 => vec![],
        1 => vec![v(&["f1"])],
        2 => vec![v(&["-"])],
        3 => vec![v(&["f1"]), v(&["f2"])],
        4 => vec![v(&["f1"]), v(&["-"])],
        _ => vec![v(&["nofile"])],
    }
}

fn mk(util: &'static str, args: Vec<Vec<String>>, stdin: Vec<u8>, files: Vec<(String, Vec<u8>)>) -> Case {
    Case { util, args, stdin, files, keep: 0 }
}

/// `mk` with `pinned` put in front of the arguments and kept while shrinking.
fn mk_keep(util: &'static str, pinned: Vec<Vec<String>>, args: Vec<Vec<String>>, stdin: Vec<u8>, files: Vec<(String, Vec<u8>)>) -> Case {
    let keep = pinned.len();
    let mut a = pinned;
    a.extend(args);
    Case { util, args: a, stdin, files, keep }
}

fn filter(g: &mut Source, util: &'static str, max: usize, table: &[OptFn]) -> Case {
    let mut a = opts(g, max, table);
    a.extend(inputs(g));
    let stdin = text(g);
    let files = std_files(g);
    mk(util, a, stdin, files)
}

// ---------------------------------------------------------------- utils

fn keydef(g: &mut Source) -> String {
    let mods = |g: &mut Source| -> String {
        let n = *g.weighted(&[(3, 0usize), (2, 1), (1, 2)]);
        (0..n).map(|_| pick(g, &["b", "d", "f", "g", "i", "M", "h", "n", "r", "V"])).collect()
    };
    let mut s = num(g, 1, 3);
    if g.bool_with(0.3) {
        s += &format!(".{}", num(g, 1, 3));
    }
    s += &mods(g);
    if g.bool_with(0.6) {
        s += &format!(",{}", num(g, 1, 3));
        if g.bool_with(0.3) {
            s += &format!(".{}", num(g, 0, 3));
        }
        s += &mods(g);
    }
    s
}

fn sort(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| one(format!("-{}", pick(g, &["b", "d", "f", "g", "i", "M", "h", "n", "r", "V", "s", "u"]))),
        |g| one(format!("-{}", pick(g, &["n", "r", "u", "f", "b"]))),
        |g| one(format!("-{}", pick(g, &["c", "C", "m", "z"]))),
        |g| vec!["-t".into(), pick(g, &[":", ",", " ", "\t"]).into()],
        |g| vec!["-k".into(), keydef(g)],
        |g| vec!["-k".into(), keydef(g)],
        |_| v(&["-o", "out"]),
        |g| one(format!("--sort={}", pick(g, &["numeric", "general-numeric", "human-numeric", "month", "version"]))),
    ];
    filter(g, "sort", 4, T)
}

fn uniq(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| one(format!("-{}", pick(g, &["c", "d", "D", "u", "i", "z"]))),
        |g| vec!["-f".into(), num(g, 0, 3)],
        |g| vec!["-s".into(), num(g, 0, 3)],
        |g| vec!["-w".into(), num(g, 0, 3)],
        |g| one(format!("--all-repeated={}", pick(g, &["none", "prepend", "separate"]))),
        |g| one(format!("--group={}", pick(g, &["separate", "prepend", "append", "both"]))),
    ];
    let mut a = opts(g, 3, T);
    match g.int(0..=3u8) {
        0 => {}
        1 => a.push(v(&["f1"])),
        2 => a.push(v(&["-"])),
        _ => a.push(v(&["f1", "out"])),
    }
    let stdin = text(g);
    let files = std_files(g);
    mk("uniq", a, stdin, files)
}

fn cutlist(g: &mut Source) -> String {
    let n = g.int(1..=3usize);
    let parts: Vec<String> = (0..n)
        .map(|_| match g.int(0..=12u8) {
            0..=2 => num(g, 1, 5),
            3..=4 => format!("{}-", num(g, 1, 5)),
            5..=6 => format!("-{}", num(g, 1, 5)),
            7..=10 => format!("{}-{}", num(g, 1, 3), num(g, 1, 5)),
            11 => "0".into(),
            _ => "".into(),
        })
        .collect();
    parts.join(",")
}

fn cut(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| vec!["-d".into(), pick(g, &[":", ",", " ", "\t", "a"]).into()],
        |_| v(&["-s"]),
        |_| v(&["--complement"]),
        |g| one(format!("--output-delimiter={}", pick(g, &["X", "", "::", "\t"]))),
        |_| v(&["-z"]),
        |_| v(&["-n"]),
    ];
    let mode = pick(g, &["-b", "-c", "-f", "-f", "-f"]);
    let mut a = vec![vec![mode.to_string(), cutlist(g)]];
    if g.bool_with(0.03) {
        a.push(vec![pick(g, &["-b", "-c", "-f"]).into(), cutlist(g)]);
    }
    a.extend(opts(g, 3, T));
    let k = g.index(a.len() + 1);
    let first = a.remove(0);
    a.insert(k.min(a.len()), first);
    a.extend(inputs(g));
    let stdin = text(g);
    let files = std_files(g);
    mk("cut", a, stdin, files)
}

fn trset(g: &mut Source) -> String {
    let n = g.int(1..=3usize);
    (0..n)
        .map(|_| {
            pick(
                g,
                &[
                    "a", "b", "c", "A", "1", " ", ":", "a-c", "A-C", "0-9", "b-z", "[:alpha:]", "[:digit:]",
                    "[:space:]", "[:upper:]", "[:lower:]", "[:punct:]", "[:alnum:]", "[:blank:]", "[:cntrl:]",
                    "[:xdigit:]", "[:print:]", "[=a=]", "[a*2]", "[b*]", "[x*3]", "\\n", "\\t", "\\\\", "\\101",
                    "\\-", "-", "ab", "xyz", "\\0", "\\177", "[", "]",
                ],
            )
            .to_string()
        })
        .collect()
}

fn tr(g: &mut Source) -> Case {
    let mut a: Vec<Vec<String>> = Vec::new();
    for f in ["-c", "-d", "-s", "-t"] {
        if g.bool_with(0.25) {
            a.push(v(&[f]));
        }
    }
    let two = g.bool_with(0.7);
    a.push(one(trset(g)));
    if two || g.bool_with(0.1) {
        a.push(one(trset(g)));
    }
    let stdin = text(g);
    mk("tr", a, stdin, vec![])
}

fn count(g: &mut Source) -> String {
    pick(g, &["0", "1", "2", "3", "5", "-1", "-2", "-3", "+0", "+1", "+2", "+3", "1b", "-0", "2K", "x"]).into()
}

fn head(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| vec!["-n".into(), count(g)],
        |g| vec!["-c".into(), count(g)],
        |g| one(format!("-{}", pick(g, &["q", "v", "z"]))),
        |g| one(format!("--lines={}", count(g))),
    ];
    filter(g, "head", 3, T)
}

fn tail(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| vec!["-n".into(), count(g)],
        |g| vec!["-c".into(), count(g)],
        |g| one(format!("-{}", pick(g, &["q", "v", "z", "r"]))),
        |g| one(format!("--bytes={}", count(g))),
    ];
    filter(g, "tail", 3, T)
}

fn wc(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| one(format!("-{}", pick(g, &["c", "m", "l", "w", "L"]))),
        |g| one(format!("--total={}", pick(g, &["auto", "always", "only", "never"]))),
    ];
    filter(g, "wc", 3, T)
}

fn nl(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| vec![pick(g, &["-b", "-h", "-f"]).into(), pick(g, &["a", "t", "n", "pa", "p^b", "p[0-9]"]).into()],
        |g| vec!["-d".into(), pick(g, &[":", "ab", "x", "::"]).into()],
        |g| vec!["-i".into(), num(g, 0, 3)],
        |g| vec!["-l".into(), num(g, 1, 3)],
        |g| vec!["-n".into(), pick(g, &["ln", "rn", "rz"]).into()],
        |_| v(&["-p"]),
        // uutils pads unnumbered lines as if the separator had length 1 (found by
        // this harness); separators of other lengths kept rare.
        |g| vec!["-s".into(), (*g.weighted(&[(8, ":"), (8, "\t"), (1, ""), (1, "  ")])).into()],
        |g| vec!["-v".into(), num(g, 0, 5)],
        |g| one(format!("--starting-line-number={}", num(g, -2, 5))),
        |g| vec!["-w".into(), num(g, 1, 4)],
    ];
    let mut c = filter(g, "nl", 4, T);
    if g.bool_with(0.3) {
        // logical page delimiters
        let mut ls: Vec<String> = String::from_utf8_lossy(&c.stdin).lines().map(String::from).collect();
        let k = g.int(1..=3usize);
        for _ in 0..k {
            let at = g.index(ls.len() + 1);
            ls.insert(at, pick(g, &["\\:\\:\\:", "\\:\\:", "\\:", ":::", "::"]).into());
        }
        c.stdin = (ls.join("\n") + "\n").into_bytes();
    }
    c
}

fn fold(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| one(format!("-{}", pick(g, &["b", "s", "c"]))),
        |g| vec!["-w".into(), num(g, 1, 8)],
        |g| vec!["-w".into(), num(g, 1, 8)],
    ];
    filter(g, "fold", 3, T)
}

fn tablist(g: &mut Source) -> String {
    // "N,/M" and "N,+M" are rejected by uutils ("tab sizes must be ascending"),
    // found by this harness; kept rare so the class does not dominate.
    match g.int(0..=40u8) {
        0..=15 => num(g, 1, 8),
        16..=27 => format!("{},{}", num(g, 1, 3), num(g, 4, 9)),
        28..=38 => format!("{},{},{}", num(g, 1, 2), num(g, 3, 5), num(g, 6, 9)),
        39 => format!("{},/{}", num(g, 1, 3), num(g, 2, 4)),
        _ => format!("{},+{}", num(g, 1, 3), num(g, 2, 4)),
    }
}

fn tabby(g: &mut Source) -> Vec<u8> {
    let n = g.int(0..=6usize);
    let ls: Vec<String> = (0..n)
        .map(|_| {
            let k = g.int(0..=6usize);
            (0..k).map(|_| pick(g, &[" ", "  ", "\t", "a", "bc", "defgh", "\t\t", "        "]).to_string()).collect()
        })
        .collect();
    finish(g, ls, 0.03)
}

/// uutils expand/unexpand drop stdin when "-" follows a file operand (found by
/// this harness, 2026-10-16); it fired on every such case, so "f1 -" is replaced.
fn no_file_then_stdin(c: &mut Case) {
    if c.args.iter().any(|a| a[0] == "-") && c.args.iter().any(|a| a[0] == "f1") {
        c.args.retain(|a| a[0] != "-");
    }
}

fn expand(g: &mut Source) -> Case {
    const T: &[OptFn] = &[|_| v(&["-i"]), |g| vec!["-t".into(), tablist(g)], |g| one(format!("--tabs={}", tablist(g)))];
    let mut c = filter(g, "expand", 2, T);
    no_file_then_stdin(&mut c);
    c.stdin = tabby(g);
    c.files[0].1 = tabby(g);
    c
}

fn unexpand(g: &mut Source) -> Case {
    const T: &[OptFn] = &[|_| v(&["-a"]), |_| v(&["--first-only"]), |g| vec!["-t".into(), tablist(g)]];
    let mut c = filter(g, "unexpand", 2, T);
    no_file_then_stdin(&mut c);
    c.stdin = tabby(g);
    c.files[0].1 = tabby(g);
    c
}

fn paste(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| vec!["-d".into(), pick(g, &[":", ",;", "\\n", "\\t", "", "\\0", "ab\\", "\\\\"]).into()],
        |_| v(&["-s"]),
        |_| v(&["-z"]),
    ];
    let mut a = opts(g, 3, T);
    let ops: &[&[&str]] = &[&["f1", "f2"], &["-", "f1"], &["f1"], &["-", "-"], &["f1", "f2", "f1"], &[]];
    a.extend(ops[g.index(ops.len())].iter().map(|s| v(&[s])));
    let stdin = text(g);
    let files = std_files(g);
    mk("paste", a, stdin, files)
}

fn comm(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| one(format!("-{}", pick(g, &["1", "2", "3", "12", "13", "23"]))),
        |_| v(&["--check-order"]),
        |_| v(&["--nocheck-order"]),
        |g| one(format!("--output-delimiter={}", pick(g, &[":", "", "XY"]))),
        |_| v(&["--total"]),
        |_| v(&["-z"]),
    ];
    let mut a = opts(g, 3, T);
    a.push(v(&["f1"]));
    a.push(v(&[pick(g, &["f2", "f2", "-"])]));
    let stdin = sorted_text(g);
    let files = vec![("f1".into(), sorted_text(g)), ("f2".into(), sorted_text(g))];
    mk("comm", a, stdin, files)
}

fn join(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| vec!["-a".into(), num(g, 1, 2)],
        |g| vec!["-e".into(), pick(g, &["X", ""]).into()],
        |_| v(&["-i"]),
        |g| vec!["-j".into(), num(g, 1, 2)],
        |g| vec!["-1".into(), num(g, 1, 3)],
        |g| vec!["-2".into(), num(g, 1, 3)],
        |g| vec!["-o".into(), pick(g, &["auto", "1.1,2.2", "0,1.2", "2.1", "0", "1.2,1.3,2.2"]).into()],
        |g| vec!["-v".into(), num(g, 1, 2)],
        |_| v(&["--nocheck-order"]),
        |_| v(&["--check-order"]),
        |_| v(&["--header"]),
        |_| v(&["-z"]),
    ];
    let mut a = opts(g, 4, T);
    let d = pick(g, &[" ", ":", "\t"]);
    if d != " " || g.bool_with(0.2) {
        a.push(vec!["-t".into(), d.into()]);
    }
    a.push(v(&["f1"]));
    a.push(v(&[pick(g, &["f2", "f2", "-"])]));
    let stdin = keyed_text(g, d);
    let files = vec![("f1".into(), keyed_text(g, d)), ("f2".into(), keyed_text(g, d))];
    mk("join", a, stdin, files)
}

fn tac(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |_| v(&["-b"]),
        |g| vec!["-s".into(), pick(g, &[":", "ab", "a", ",", " "]).into()],
        |g| vec!["-s".into(), pick(g, &["[0-9]", "a*b", "[:,]", "x\\|y"]).into(), "-r".into()],
    ];
    filter(g, "tac", 2, T)
}

fn od(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| one(format!("-A{}", pick(g, &["d", "o", "x", "n"]))),
        |g| vec!["-j".into(), pick(g, &["0", "1", "3", "8", "0x2", "1b", "100"]).into()],
        |g| vec!["-N".into(), pick(g, &["0", "1", "3", "5", "16", "0x4"]).into()],
        |g| vec!["-S".into(), pick(g, &["1", "2", "3", "5"]).into()],
        |g| {
            let n = g.int(1..=2usize);
            let t: String = (0..n)
                .map(|_| {
                    pick(g, &["a", "c", "d1", "d2", "d4", "d8", "o1", "o2", "o4", "u1", "u2", "u4", "u8", "x1", "x2", "x4", "x8", "dC", "dS", "dI", "dL", "x", "o", "d", "u", "a", "c"])
                        .to_string()
                })
                .collect();
            let z = if g.bool_with(0.2) { "z" } else { "" };
            vec!["-t".into(), format!("{t}{z}")]
        },
        |_| v(&["-v"]),
        |g| one(format!("-w{}", pick(g, &["1", "2", "4", "8", "16", "3", "32"]))),
        |g| one(format!("-{}", pick(g, &["b", "c", "d", "o", "s", "x", "i", "l"]))),
        |g| one(format!("--endian={}", pick(g, &["big", "little"]))),
    ];
    filter(g, "od", 4, T)
}

fn numline(g: &mut Source, d: &str) -> String {
    let n = g.int(1..=3usize);
    (0..n)
        .map(|_| {
            pick(
                g,
                &[
                    "1", "0", "10", "999", "1000", "1001", "1023", "1024", "1025", "1.5K", "2Mi", "3G", "-3", "-1000",
                    "12345678", "0.5", "1.25", "99.95", "-0.5", "abc", "1e3", "5T", "7Ki", "4096", "1048576",
                    "123456789012", "", "2k", "1.5", "  42", "8.7654",
                ],
            )
            .to_string()
        })
        .collect::<Vec<_>>()
        .join(d)
}

fn numfmt(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| one(format!("--from={}", pick(g, &["none", "auto", "si", "iec", "iec-i"]))),
        |g| one(format!("--to={}", pick(g, &["none", "si", "iec", "iec-i", "auto"]))),
        |g| one(format!("--to={}", pick(g, &["si", "iec", "iec-i"]))),
        |g| one(format!("--padding={}", num(g, -6, 8))),
        |g| one(format!("--round={}", pick(g, &["up", "down", "from-zero", "towards-zero", "nearest"]))),
        |g| one(format!("--suffix={}", pick(g, &["B", "b/s", ""]))),
        |g| one(format!("--format={}", pick(g, &["%f", "%10f", "%-8f", "%.1f", "%.3f", "%08.2f", "%.0f", "x%fy", "%'f", "%5.1f", "%f%%"]))),
        |g| one(format!("--field={}", pick(g, &["1", "2", "2-", "-2", "1,3", "-"]))),
        |g| vec!["-d".into(), pick(g, &[":", " ", ","]).into()],
        |g| one(format!("--header{}", pick(g, &["", "=1", "=2"]))),
        |g| one(format!("--invalid={}", pick(g, &["abort", "fail", "warn", "ignore"]))),
        |g| one(format!("--to-unit={}", pick(g, &["1", "1000", "1024", "10"]))),
        |g| one(format!("--from-unit={}", pick(g, &["1", "1000", "1024", "10"]))),
        |_| v(&["-z"]),
    ];
    let mut a = opts(g, 4, T);
    let mut stdin = Vec::new();
    if g.bool_with(0.3) {
        let n = g.int(1..=3usize);
        for _ in 0..n {
            a.push(one(numline(g, " ").trim().to_string()));
        }
    } else {
        let n = g.int(0..=5usize);
        // numfmt re-joins tab-separated fields with tabs, GNU with spaces: a
        // frequent difference seen at intake, so tab-separated input stays rare.
        let d = *g.weighted(&[(30, " "), (15, "  "), (10, ":")]);
        let ls: Vec<String> = (0..n).map(|_| numline(g, d)).collect();
        stdin = finish(g, ls, 0.0);
    }
    mk("numfmt", a, stdin, vec![])
}

fn seq(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| vec!["-s".into(), pick(g, &[",", "", ":", "\\n", " - "]).into()],
        |_| v(&["-w"]),
        |g| vec!["-f".into(), pick(g, &["%g", "%.2f", "%e", "%03g", "x%gy", "%5.1f", "%-4g|", "%G", "%E", "%%%g"]).into()],
        |g| one(format!("--separator={}", pick(g, &["\t", "ab"]))),
    ];
    let vals = &["0", "1", "2", "3", "-1", "5", "10", "0.5", "1.5", "-2.5", "1e1", "0.1", "007", "+3", "-0", "1.00", "2.50", "0x10", "1e-1"];
    let mut a = opts(g, 2, T);
    let n = *g.weighted(&[(2, 1usize), (3, 2), (3, 3)]);
    for _ in 0..n {
        let x = pick(g, vals);
        a.push(one(x.into()));
    }
    // "-1" as the first operand is an operand, not an option, only after "--" in GNU's getopt.
    mk("seq", a, vec![], vec![])
}

fn printf_fmt(g: &mut Source) -> String {
    let n = g.int(1..=4usize);
    let mut s = String::new();
    for _ in 0..n {
        match g.int(0..=9u8) {
            0 => s.push_str(pick(g, &["a", "x=", " ", "--", "ab c"])),
            1 => s.push_str(pick(g, &["\\n", "\\t", "\\\\", "\\101", "\\0101", "\\x41", "\\c", "\\e", "\\a", "%%", "\\u0041", "\\q"])),
            _ => {
                s.push('%');
                let k = *g.weighted(&[(4, 0usize), (2, 1), (1, 2)]);
                for _ in 0..k {
                    s.push_str(pick(g, &["-", "+", " ", "#", "0"]));
                }
                match g.int(0..=4u8) {
                    0 => s.push_str(&num(g, 0, 8)),
                    1 => s.push('*'),
                    _ => {}
                }
                match g.int(0..=4u8) {
                    0 => s.push_str(&format!(".{}", num(g, 0, 6))),
                    1 => s.push_str(".*"),
                    _ => {}
                }
                s.push_str(pick(g, &["d", "i", "o", "u", "x", "X", "c", "s", "b", "q", "f", "e", "g", "E", "G", "s", "d"]));
            }
        }
    }
    s
}

fn printf(g: &mut Source) -> Case {
    let mut a = vec![one(printf_fmt(g))];
    let n = g.int(0..=4usize);
    for _ in 0..n {
        a.push(one(
            pick(
                g,
                &[
                    "1", "-2", "0x1F", "010", "'a", "\"b", "3.5", "1e2", "abc", "", "+7", "9223372036854775807",
                    "9223372036854775808", "-1.5", "0", " 3", "3 ", "a\\nb", "\\0101", "\\c", "1.", ".5", "0.125",
                    "-0", "18446744073709551615", "1e400", "x y", "5", "42",
                ],
            )
            .into(),
        ));
    }
    mk("printf", a, vec![], vec![])
}

fn expr_tree(g: &mut Source, depth: u32, out: &mut Vec<String>) {
    let leaf = |g: &mut Source, out: &mut Vec<String>| {
        out.push(pick(g, &["0", "1", "2", "-3", "10", "abc", "a", "", "007", "b", "9223372036854775807", "-0", "ab1"]).into())
    };
    if depth == 0 || g.bool_with(0.3) {
        return leaf(g, out);
    }
    match g.int(0..=9u8) {
        0..=5 => {
            let paren = g.bool_with(0.2);
            if paren {
                out.push("(".into());
            }
            expr_tree(g, depth - 1, out);
            out.push(pick(g, &["|", "&", "=", ">", ">=", "<", "<=", "!=", "+", "-", "*", "/", "%", ":", "+", "-"]).into());
            if out.last().map(|s| s == ":").unwrap_or(false) {
                out.push(pick(g, &["a*", "\\(a*\\)", ".*", "[0-9]*", "^a", "\\(.\\)\\(.\\)", "a\\|b", "x", "[ab]\\+", "\\{1\\}"]).into());
            } else {
                expr_tree(g, depth - 1, out);
            }
            if paren {
                out.push(")".into());
            }
        }
        6 => {
            out.push("length".into());
            leaf(g, out)
        }
        7 => {
            out.push("substr".into());
            leaf(g, out);
            out.push(num(g, -1, 4));
            out.push(num(g, -1, 4));
        }
        8 => {
            out.push("index".into());
            leaf(g, out);
            out.push(pick(g, &["a", "b1", "", "xyz"]).into());
        }
        _ => {
            out.push("match".into());
            leaf(g, out);
            out.push(pick(g, &["a*", "\\(a*\\)", ".*", "[0-9]*"]).into());
        }
    }
}

fn expr(g: &mut Source) -> Case {
    let mut toks = Vec::new();
    expr_tree(g, 3, &mut toks);
    mk("expr", toks.into_iter().map(one).collect(), vec![], vec![])
}

fn test_prim(g: &mut Source, out: &mut Vec<String>) {
    match g.int(0..=5u8) {
        0 => {
            out.push(pick(g, &["-n", "-z"]).into());
            out.push(pick(g, &["", "a", "-n", "="]).into());
        }
        1 => {
            out.push(pick(g, &["-e", "-f", "-d", "-s", "-r", "-w", "-x", "-L", "-h", "-p", "-S", "-b", "-c"]).into());
            out.push(pick(g, &["f1", "f2", "d1", "nofile", "", "l1"]).into());
        }
        2 | 3 => {
            out.push(pick(g, &["1", "-1", "+1", " 2", "02", "a", "", "9223372036854775808", "0"]).into());
            out.push(pick(g, &["-eq", "-ne", "-lt", "-le", "-gt", "-ge"]).into());
            out.push(pick(g, &["1", "-1", "2 ", "0", "x", "-0"]).into());
        }
        4 => {
            out.push(pick(g, &["a", "b", "", "-n", "!", "(", "="]).into());
            out.push(pick(g, &["=", "!=", "==", "<", ">"]).into());
            out.push(pick(g, &["a", "b", "", ")"]).into());
        }
        _ => out.push(pick(g, &["a", "", "-n", "!", "(", ")", "-a", "-o", "f1"]).into()),
    }
}

fn test(g: &mut Source) -> Case {
    let mut toks = Vec::new();
    let n = g.int(1..=3usize);
    for i in 0..n {
        if i > 0 {
            toks.push(pick(g, &["-a", "-o"]).into());
        }
        if g.bool_with(0.15) {
            toks.push("!".into());
        }
        let paren = g.bool_with(0.1);
        if paren {
            toks.push("(".into());
        }
        test_prim(g, &mut toks);
        if paren {
            toks.push(")".into());
        }
    }
    let files = vec![("f1".into(), b"x\n".to_vec()), ("f2".into(), vec![])];
    let mut c = mk("test", toks.into_iter().map(one).collect(), vec![], files);
    c.files.push(("d1/".into(), vec![]));
    c
}

fn path(g: &mut Source) -> String {
    let n = g.int(0..=4usize);
    (0..n).map(|_| pick(g, &["a", "b.txt", ".", "..", "/", "//", "c.d.e", "", ".x", "a/"]).to_string()).collect::<Vec<_>>().join("/")
}

fn basename(g: &mut Source) -> Case {
    let mut a = Vec::new();
    let mut multi = false;
    for f in ["-a", "-z"] {
        if g.bool_with(0.2) {
            multi |= f == "-a";
            a.push(v(&[f]));
        }
    }
    if g.bool_with(0.2) {
        multi = true;
        a.push(vec!["-s".into(), pick(g, &[".txt", "e", "b.txt", ""]).into()]);
    }
    let n = if multi { g.int(1..=3usize) } else { *g.weighted(&[(5, 1usize), (2, 2), (1, 3)]) };
    for _ in 0..n {
        a.push(one(path(g)));
    }
    mk("basename", a, vec![], vec![])
}

fn dirname(g: &mut Source) -> Case {
    let mut a = Vec::new();
    if g.bool_with(0.2) {
        a.push(v(&["-z"]));
    }
    let n = g.int(1..=3usize);
    for _ in 0..n {
        a.push(one(path(g)));
    }
    mk("dirname", a, vec![], vec![])
}

fn rand_bytes(g: &mut Source) -> Vec<u8> {
    let n = g.int(0..=20usize);
    (0..n).map(|_| *g.pick(&[0u8, 1, b'a', b'z', b'\n', 0xff, 0x80, b'=', 0x3f, b'~'])).collect()
}

fn b64(data: &[u8], alpha: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for ch in data.chunks(3) {
        let b = [ch[0], *ch.get(1).unwrap_or(&0), *ch.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= ch.len() {
                out.push(alpha[((n >> (18 - 6 * i)) & 63) as usize]);
            } else {
                out.push(b'=');
            }
        }
    }
    out
}

fn b32(data: &[u8], alpha: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for ch in data.chunks(5) {
        let mut buf = [0u8; 5];
        buf[..ch.len()].copy_from_slice(ch);
        let n = buf.iter().fold(0u64, |a, &b| (a << 8) | u64::from(b));
        let used = [0, 2, 4, 5, 7, 8][ch.len()];
        for i in 0..8 {
            out.push(if i < used { alpha[((n >> (35 - 5 * i)) & 31) as usize] } else { b'=' });
        }
    }
    out
}

const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const B64URL: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const B32: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
const B32HEX: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUV";

/// A valid encoding of random bytes, sometimes wrapped, sometimes damaged.
fn encoded(g: &mut Source, kind: &str) -> Vec<u8> {
    let data = rand_bytes(g);
    let mut e = match kind {
        "b64" => b64(&data, B64),
        "b64url" => b64(&data, B64URL),
        "b32" => b32(&data, B32),
        "b32hex" => b32(&data, B32HEX),
        _ => data.iter().map(|b| format!("{b:02X}")).collect::<String>().into_bytes(),
    };
    if g.bool_with(0.3) && !e.is_empty() {
        let w = g.int(1..=8usize);
        let mut x = Vec::new();
        for (i, c) in e.iter().enumerate() {
            if i > 0 && i % w == 0 {
                x.push(b'\n');
            }
            x.push(*c);
        }
        e = x;
    }
    if g.bool_with(0.15) && !e.is_empty() {
        let at = g.index(e.len());
        e[at] = *g.pick(&[b'!', b'=', b'\n', b'a', b' ', b'-']);
    }
    if g.bool_with(0.7) {
        e.push(b'\n');
    }
    e
}

fn base64(g: &mut Source) -> Case {
    base_n(g, "base64", "b64", &[])
}
fn base32(g: &mut Source) -> Case {
    base_n(g, "base32", "b32", &[])
}
fn basenc(g: &mut Source) -> Case {
    let (flag, kind) = *g.pick(&[
        ("--base64", "b64"),
        ("--base64url", "b64url"),
        ("--base32", "b32"),
        ("--base32hex", "b32hex"),
        ("--base16", "b16"),
        ("--base2msbf", "raw"),
        ("--base2lsbf", "raw"),
        ("--z85", "raw"),
    ]);
    base_n(g, "basenc", kind, &[flag])
}

fn base_n(g: &mut Source, util: &'static str, kind: &str, pre: &[&str]) -> Case {
    let mut a: Vec<Vec<String>> = pre.iter().map(|s| v(&[s])).collect();
    let decode = g.bool_with(0.5);
    if decode {
        a.push(v(&[pick(g, &["-d", "--decode", "-d"])]));
        if g.bool_with(0.2) {
            a.push(v(&["-i"]));
        }
    }
    if g.bool_with(0.3) {
        a.push(vec!["-w".into(), pick(g, &["0", "1", "4", "7", "76"]).into()]);
    }
    let stdin = if decode && kind != "raw" { encoded(g, kind) } else if g.bool() { rand_bytes(g) } else { text(g) };
    let use_file = g.bool_with(0.2);
    if use_file {
        a.push(v(&["f1"]));
    }
    let files = vec![("f1".into(), stdin.clone())];
    mk(util, a, stdin, files)
}

fn cksum(g: &mut Source) -> Case {
    let util = *g.pick(&["cksum", "cksum", "md5sum", "sha1sum", "sha256sum", "b2sum", "sum", "sha512sum"]);
    let mut a: Vec<Vec<String>> = Vec::new();
    match util {
        "cksum" => {
            const T: &[OptFn] = &[
                |g| vec!["-a".into(), pick(g, &["crc", "crc32b", "md5", "sha1", "sha256", "sha512", "blake2b", "sm3", "bsd", "sysv", "sha224", "sha384"]).into()],
                |g| vec!["-a".into(), pick(g, &["md5", "sha256", "blake2b", "crc"]).into()],
                |_| v(&["--untagged"]),
                |_| v(&["--tag"]),
                |_| v(&["--base64"]),
                |_| v(&["-z"]),
                |g| vec!["-l".into(), pick(g, &["8", "16", "128", "256", "512", "0", "7"]).into()],
            ];
            a = opts(g, 3, T);
        }
        "sum" => {
            if g.bool_with(0.5) {
                a.push(v(&[pick(g, &["-r", "-s"])]));
            }
        }
        _ => {
            const T: &[OptFn] = &[|_| v(&["--tag"]), |_| v(&["-z"]), |_| v(&["-b"]), |_| v(&["-t"])];
            a = opts(g, 2, T);
        }
    }
    a.extend(inputs(g));
    let stdin = text(g);
    let files = std_files(g);
    mk(util, a, stdin, files)
}

fn factor(g: &mut Source) -> Case {
    let mut a = Vec::new();
    if g.bool_with(0.1) {
        a.push(v(&["-h"]));
    }
    let n = g.int(0..=3usize);
    let mut stdin = Vec::new();
    let mut nums = Vec::new();
    for _ in 0..n {
        let x = match g.int(0..=9u8) {
            0..=3 => g.int(0u64..=10_000).to_string(),
            4 => g.int(0u64..=u64::MAX).to_string(),
            5 => (u128::from(g.int(0u64..=u64::MAX)) * u128::from(g.int(1u64..=1 << 30))).to_string(),
            6 => pick(g, &["18446744073709551615", "18446744073709551616", "4294967296", "1", "0", "-1", "abc", "+5", " 7", "3.0", "0x10", "340282366920938463463374607431768211455"]).into(),
            7 => (u64::from(g.int(2u32..=65535)) * u64::from(g.int(2u32..=65535))).to_string(),
            _ => g.int(0u64..=1u64 << 40).to_string(),
        };
        nums.push(x);
    }
    if g.bool_with(0.3) {
        stdin = (nums.join(pick(g, &[" ", "\n", "\t"])) + "\n").into_bytes();
    } else {
        a.extend(nums.into_iter().map(one));
    }
    mk("factor", a, stdin, vec![])
}

fn split(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| vec!["-l".into(), (*g.weighted(&[(10, "1"), (10, "2"), (10, "3"), (3, "0"), (1, "1k")])).into()],
        |g| vec!["-b".into(), pick(g, &["1", "3", "5", "10", "0", "1K", "2b"]).into()],
        |g| vec!["-C".into(), pick(g, &["1", "4", "8", "20"]).into()],
        |g| vec!["-n".into(), pick(g, &["1", "2", "3", "l/2", "r/2", "1/2", "2/3", "l/1/2", "r/2/3", "5", "l/3", "0", "4/3"]).into()],
        |g| vec!["-a".into(), pick(g, &["1", "2", "3", "0"]).into()],
        |_| v(&["-d"]),
        |_| v(&["-x"]),
        |g| one(format!("--numeric-suffixes={}", pick(g, &["0", "1", "9", "98"]))),
        |g| one(format!("--hex-suffixes={}", pick(g, &["0", "15"]))),
        |g| one(format!("--additional-suffix={}", pick(g, &[".t", "x"]))),
        |_| v(&["-e"]),
        |g| vec!["-t".into(), pick(g, &[":", ",", "\\0", "a"]).into()],
        |_| v(&["--verbose"]),
    ];
    let mut a = opts(g, 4, T);
    if g.bool_with(0.005) {
        a.push(v(&["-u"])); // uutils rejects -u (found here)
    }
    match g.int(0..=3u8) {
        0 => {}
        1 => a.push(v(&["f1"])),
        2 => a.push(v(&["-", "p"])),
        _ => a.push(v(&["f1", "p"])),
    }
    let stdin = text(g);
    let files = vec![("f1".into(), text(g))];
    mk("split", a, stdin, files)
}

fn csplit(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| vec!["-f".into(), pick(g, &["p", "x-"]).into()],
        |g| vec!["-b".into(), pick(g, &["%02d", "%x", "%03o.t", "a%d"]).into()],
        |g| vec!["-n".into(), num(g, 1, 3)],
        |_| v(&["-k"]),
        |_| v(&["-s"]),
        |_| v(&["-z"]),
        |_| v(&["--suppress-matched"]),
    ];
    let mut a = opts(g, 3, T);
    a.push(v(&[pick(g, &["f1", "f1", "-"])]));
    let n = g.int(1..=3usize);
    for _ in 0..n {
        let p = match g.int(0..=6u8) {
            0..=1 => num(g, 1, 6),
            2..=3 => format!("/{}/{}", pick(g, &["a", "^b", "1", "x$", "^$", "[0-9]"]), pick(g, &["", "", "+1", "-1", "+2"])),
            4 => format!("%{}%{}", pick(g, &["a", "^b", "1"]), pick(g, &["", "+1", "-1"])),
            5 => format!("{{{}}}", pick(g, &["1", "2", "*", "0"])),
            _ => pick(g, &["0", "/a", "{x}", "%"]).into(),
        };
        a.push(one(p));
    }
    let stdin = text(g);
    let files = vec![("f1".into(), text(g))];
    mk("csplit", a, stdin, files)
}

fn truncate(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| {
            let p = pick(g, &["", "", "+", "-", "<", ">", "/", "%"]);
            vec!["-s".into(), format!("{p}{}", pick(g, &["0", "1", "3", "10", "1K", "2b", "1KB", "x", "-2"]))]
        },
        |_| v(&["-c"]),
        |_| v(&["-o"]),
        |_| v(&["-r", "f2"]),
    ];
    let mut a = opts(g, 3, T);
    let ops: &[&[&str]] = &[&["f1"], &["f3"], &["f1", "f3"], &[]];
    a.extend(ops[g.index(ops.len())].iter().map(|s| v(&[s])));
    let files = std_files(g);
    mk("truncate", a, vec![], files)
}

fn cat(g: &mut Source) -> Case {
    const T: &[OptFn] = &[|g| one(format!("-{}", pick(g, &["A", "b", "e", "E", "n", "s", "t", "T", "u", "v"])))];
    let mut c = filter(g, "cat", 3, T);
    if g.bool_with(0.5) {
        let ls = vec!["".to_string(), "".to_string()];
        let mut extra = finish(g, ls, 0.0);
        extra.extend(std::iter::repeat_n(*g.pick(CTRL), g.int(1..=2usize)));
        extra.extend([0x80, 0xff, 0xa0][..g.int(0..=3usize)].iter());
        c.stdin.extend(extra);
    }
    c
}

fn echo(g: &mut Source) -> Case {
    let mut a = Vec::new();
    let n = g.int(0..=2usize);
    for _ in 0..n {
        a.push(v(&[pick(g, &["-n", "-e", "-E", "-ne", "-en", "-x", "--", "-neE"])]));
    }
    let m = g.int(0..=3usize);
    for _ in 0..m {
        a.push(one(pick(g, &["a", "\\n", "b\\tc", "\\0101", "\\x41", "\\c", "x\\cy", "\\\\", "\\e", "-n", "", "\\u263a", "\\07", "\\x"]).into()));
    }
    mk("echo", a, vec![], vec![])
}

fn fmt(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| one(format!("-w{}", num(g, 3, 30))),
        |g| vec!["-g".into(), num(g, 2, 20)],
        |g| one(format!("-{}", pick(g, &["s", "u", "t", "c"]))),
        |g| vec!["-p".into(), pick(g, &["a", "#", " "]).into()],
    ];
    let mut c = filter(g, "fmt", 3, T);
    // weights: fmt, comm, join, tr, expand, unexpand differ from GNU on several
    // common inputs (found by this harness, see README); weight 1 keeps the
    // overall failure rate low while each class still shows up within minutes.
    // Leading blanks mixing spaces and tabs are handled differently (intake: 27 of
    // 65 ASCII diffs were fmt); keep tabs out of most fmt inputs.
    if g.bool_with(0.98) {
        for b in c.stdin.iter_mut().chain(c.files.iter_mut().flat_map(|f| f.1.iter_mut())) {
            if *b == b'\t' {
                *b = b' ';
            }
        }
    }
    c
}

fn dd(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| one(format!("bs={}", pick(g, &["1", "2", "3", "5", "1K", "2x2", "0"]))),
        |g| one(format!("ibs={}", pick(g, &["1", "2", "3", "7"]))),
        |g| one(format!("obs={}", pick(g, &["1", "2", "4", "7"]))),
        |g| one(format!("count={}", pick(g, &["0", "1", "2", "3", "5B"]))),
        |g| one(format!("skip={}", pick(g, &["0", "1", "2", "4", "3B"]))),
        |g| one(format!("seek={}", *g.weighted(&[(5, "0"), (5, "1"), (5, "2"), (1, "5B")]))),
        |g| one(format!("conv={}", pick(g, &["ucase", "lcase", "swab", "notrunc", "sync", "block", "unblock", "noerror", "ucase,swab", "sync,block", "excl", "nocreat", "fdatasync"]))),
        |g| one(format!("cbs={}", pick(g, &["1", "3", "4", "8"]))),
        |g| one(format!("iflag={}", pick(g, &["skip_bytes", "count_bytes", "fullblock", "skip_bytes,count_bytes"]))),
        |g| one(format!("oflag={}", pick(g, &["seek_bytes", "append"]))),
    ];
    let mut a = opts(g, 4, T);
    a.push(one("status=none".into()));
    if g.bool_with(0.85) {
        a.push(one("if=f1".into()));
    }
    if g.bool_with(0.5) {
        a.push(one(pick(g, &["of=f2", "of=f3"]).into()));
    }
    let stdin = text(g);
    let files = std_files(g);
    mk("dd", a, stdin, files)
}

// ---------------------------------------------------------------- date

/// Absolute dates only (or @epoch): a date string without a calendar date is
/// relative to "now", which the two sides read at slightly different times.
const DATE_ABS: &[&str] = &[
    "2024-01-01", "2024-02-29", "1970-01-01", "2000-12-31", "1999-06-15", "2024-03-10", "2038-01-19", "1969-12-31",
    "Jan 5 2020", "5 Jan 2020", "1/5/2020", "20200105", "2020-1-5", "December 31, 1999", "29 feb 2023", "2024-13-01",
    "0001-01-01", "9999-12-31", "2024-12-31T23:59:59", "Sun, 05 Jan 2020", "1972-06-30",
];
const DATE_TIME: &[&str] = &[
    "00:00", "12:00", "13:45:01", "23:59:59", "23:59:60", "3pm", "12am", "12pm", "noon", "midnight", "08:30:00.5",
    "1:02:03.123456789", "24:00", "10:11:12", "7am", "11:59 PM",
];
const DATE_ZONE: &[&str] = &["UTC", "Z", "GMT", "+0530", "-08:00", "+1", "-0000", "EST", "PDT", "CET", "UTC+3", "J", "+14", "-1200"];
const DATE_REL: &[&str] = &[
    "+1 day", "-1 day", "1 day ago", "2 weeks", "3 months", "-1 year", "next monday", "last friday", "+36 hours",
    "1 fortnight", "tomorrow", "yesterday", "+90 minutes", "5 seconds ago", "today", "this thursday", "1 month ago",
    "+12 months", "-2 hours", "next year", "last day", "1 week ago", "+0 days", "sunday", "next sec",
];
const DATE_EPOCH: &[&str] = &["@0", "@1700000000", "@-86400", "@1.5", "@1700000000.123456789", "@-1", "@2147483648", "@ 5", "@1e3", "@-0.25"];

fn date_str(g: &mut Source) -> String {
    if g.bool_with(0.15) {
        return pick(g, DATE_EPOCH).into();
    }
    let mut p: Vec<String> = Vec::new();
    if g.bool_with(0.08) {
        // POSIX TZ strings (EST5EDT, UTC-2) are rejected by uutils: one class, rare
        let tz = if g.bool_with(0.15) { pick(g, &["EST5EDT", "UTC-2", "<+03>-3"]) } else { pick(g, &["Europe/Paris", "America/New_York", "UTC", "Asia/Kolkata", "Australia/Lord_Howe", "Pacific/Chatham"]) };
        p.push(format!("TZ=\"{tz}\""));
    }
    p.push(pick(g, DATE_ABS).into());
    if g.bool_with(0.6) {
        p.push(pick(g, DATE_TIME).into());
        if g.bool_with(0.25) {
            p.push(pick(g, DATE_ZONE).into());
        }
    }
    for _ in 0..*g.weighted(&[(5, 0usize), (4, 1), (1, 2)]) {
        let mut r = pick(g, DATE_REL);
        // a signed number right after a time of day is a zone correction in GNU
        // ("12:00 +1 day" is 11:00 the next day); uutils reads a relative item
        let after_time = p.last().map_or(false, |l| DATE_TIME.contains(&l.as_str()));
        if after_time && !g.bool_with(0.1) {
            while r.starts_with(['+', '-']) {
                r = pick(g, DATE_REL);
            }
        }
        p.push(r.into());
    }
    p.join(" ")
}

/// strftime-like format: literals and conversions with optional flag and width
/// (the E and O modifiers are missing, #10958).
fn date_fmt(g: &mut Source) -> String {
    let mut s = String::from("+");
    for _ in 0..g.int(1..=4usize) {
        if g.bool_with(0.2) {
            s.push_str(pick(g, &[" ", "x", "-", ":", "T", "%%", "%n", "%t"]));
            continue;
        }
        s.push('%');
        if g.bool_with(0.2) {
            s.push_str(pick(g, &["-", "_", "0", "^", "#", "+"]));
        }
        if g.bool_with(0.1) {
            s.push_str(&num(g, 0, 12));
        }
        s.push_str(pick(
            g,
            &[
                "a", "A", "b", "B", "c", "C", "d", "D", "e", "F", "g", "G", "h", "H", "I", "j", "k", "l", "m", "M", "N", "p",
                "P", "q", "r", "R", "s", "S", "T", "u", "U", "V", "w", "W", "x", "X", "y", "Y", "z", ":z", "::z", ":::z", "Z",
                "3N", "s", "F", "T",
            ],
        ));
    }
    s
}

fn date(g: &mut Source) -> Case {
    let mut a: Vec<Vec<String>> = Vec::new();
    let mut pin: Vec<Vec<String>> = Vec::new();
    if g.bool_with(0.3) {
        a.push(v(&["-u"]));
    }
    let mut files = Vec::new();
    if g.bool_with(0.12) {
        // -f: one date per line (blank lines are midnight *today*: avoided, and #14673)
        let n = g.int(1..=3usize);
        let mut b = (0..n).map(|_| date_str(g)).collect::<Vec<_>>().join("\n");
        b.push('\n');
        files.push(("f1".to_string(), b.into_bytes()));
        pin.push(v(&["-f", "f1"]));
    } else {
        let d = date_str(g);
        pin.push(if g.bool_with(0.5) { vec!["-d".into(), d] } else { one(format!("--date={d}")) });
    }
    match *g.weighted(&[(6, 0), (1, 1), (1, 2), (1, 3), (1, 4)]) {
        0 => a.push(one(date_fmt(g))),
        1 => a.push(one(format!("-I{}", pick(g, &["", "date", "hours", "minutes", "seconds", "ns"])))),
        2 => a.push(v(&["-R"])),
        3 => a.push(one(format!("--rfc-3339={}", pick(g, &["date", "seconds", "ns"])))),
        _ => {}
    }
    mk_keep("date", pin, a, vec![], files)
}

// ---------------------------------------------------------------- pr, ptx, tsort

fn pr(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| one(format!("-{}", pick(g, &["2", "3", "a", "d", "F", "f", "J", "r", "d", "r"]))),
        // a bare -n takes the next operand as its argument in uutils (#10302)
        |g| one(pick(g, &["-e", "-e4", "-e:3", "-n:3", "-n.2", "-n:", "-s", "-s:", "-S", "-S::"]).into()),
        // -v and -i are rejected by uutils (unexpected argument): rare
        |g| if g.bool_with(0.1) { one(pick(g, &["-v", "-i", "-i8", "-i,4"]).into()) } else { v(&["-r"]) },
        |g| one(pick(g, &["+2", "+1:2", "--pages=2", "--pages=1:1", "+3"]).into()),
        |g| one(format!("--columns={}", num(g, 1, 4))),
        |g| vec!["-l".into(), num(g, 1, 20)],
        |g| vec!["-w".into(), num(g, 8, 40)],
        // uutils pads every line to the -W width (GNU only truncates): rare
        |g| if g.bool_with(0.2) { vec!["-W".into(), num(g, 8, 40)] } else { vec!["-l".into(), num(g, 5, 70)] },
        |g| vec!["-o".into(), num(g, 0, 4)],
        |g| one(format!("--first-line-number={}", num(g, 0, 9))),
        |g| one(pick(g, &["-T", "-t", "--omit-pagination", "--omit-header"]).into()),
        |g| one(pick(g, &["-e", "-n5", "-s", "-d"]).into()),
    ];
    let mut a = opts(g, 4, T);
    // GNU pads multi-column output (-COLUMN, -m) with tabs, uutils with spaces:
    // nearly every such case differs, so they are rare here.
    let multi = |o: &Vec<String>| matches!(o[0].as_str(), "-2" | "-3") || o[0].starts_with("--columns");
    if a.iter().any(multi) && !g.bool_with(0.15) {
        a.retain(|o| !multi(o));
    }
    // headers carry a date: the file's mtime or now. Fixed --date-format instead.
    let mut pin = Vec::new();
    if g.bool_with(0.6) {
        pin.push(one(pick(g, &["-t", "-t", "-T", "--omit-header", "--omit-pagination"]).into()));
    } else {
        pin.push(one(format!("--date-format={}", pick(g, &["D", "%%", "x y"]))));
        if g.bool_with(0.5) {
            a.push(vec!["-h".into(), pick(g, &["H", "", "a long header text"]).into()]);
        }
    }
    if g.bool_with(0.03) {
        a.push(v(&["-m"]));
        a.push(v(&["f1"]));
        a.push(v(&["f2"]));
    } else {
        a.extend(inputs(g));
    }
    let stdin = tabby(g);
    let files = vec![("f1".into(), tabby(g)), ("f2".into(), text(g))];
    a.retain(|o| !pin.contains(o));
    mk_keep("pr", pin, a, stdin, files)
}

fn ptx(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| one(format!("-{}", pick(g, &["f", "T", "O"]))),
        // each of these differs on nearly every input (-S is not honoured at all,
        // -W '\w+', -A and -R in GNU mode, -r): rare
        |g| {
            if g.bool_with(0.1) {
                match g.int(0..=3u32) {
                    0 => vec!["-S".into(), pick(g, &["\\.", "[.?!]"]).into()],
                    1 => vec!["-W".into(), "\\w+".into()],
                    _ => one(format!("-{}", pick(g, &["A", "R", "r"]))),
                }
            } else {
                v(&["-T"])
            }
        },
        |g| vec!["-g".into(), num(g, 1, 5)],
        |g| if g.bool_with(0.3) { vec!["-w".into(), num(g, 10, 60)] } else { v(&["-f"]) },
        // (GNU -G rejects a -S regex that matches a whole line: "match of length zero")
        |_| vec!["-S".into(), "$".into()],
        |g| vec!["-W".into(), pick(g, &["[a-z]+", "[^ ]+", "[0-9]"]).into()],
        |g| vec!["-F".into(), pick(g, &["/", "...", ""]).into()],
        |_| v(&["-i", "f2"]),
        |_| v(&["-o", "f2"]),
    ];
    let mut a = opts(g, 3, T);
    // GNU mode treats the whole input as one text across lines; uutils works per
    // line (#9335): a broad class. Traditional mode (-G) mostly, else one line.
    let gnu_mode = !g.bool_with(0.75);
    let pin = if gnu_mode { vec![] } else { vec![v(&["-G"])] };
    // Leading, trailing and repeated blanks are kept in GNU's contexts and trimmed
    // or marked as truncated by uutils: frequent, so words are mostly joined by
    // single spaces here.
    let plain = g.bool_with(0.85);
    let mk_line = |g: &mut Source| -> String {
        if plain {
            let n = g.int(1..=4usize);
            (0..n).map(|_| pick(g, &["a", "b", "ab", "x", "10", "the", "1.5", "c.", "Jan", "zz", "B"])).collect::<Vec<_>>().join(" ")
        } else {
            line(g, " ")
        }
    };
    let mk_text = |g: &mut Source| -> Vec<u8> {
        if gnu_mode {
            let l = mk_line(g);
            finish(g, vec![l], 0.0)
        } else {
            // GNU -G fails on an empty line ("regular expression has a match of length zero")
            let n = g.int(1..=6usize);
            let mut ls: Vec<String> = (0..n).map(|_| mk_line(g)).collect();
            ls.retain(|l| !l.is_empty());
            if ls.is_empty() {
                ls.push("x y".into());
            }
            finish(g, ls, 0.0)
        }
    };
    let stdin = mk_text(g);
    let f1 = mk_text(g);
    let words = ["a", "b", "ab", "x", "the", "10"];
    let n = g.int(0..=3usize);
    let mut f2: Vec<u8> = (0..n).flat_map(|_| format!("{}\n", words[g.index(words.len())]).into_bytes()).collect();
    if f2.is_empty() && g.bool_with(0.5) {
        f2 = b"a\n".to_vec();
    }
    if g.bool_with(0.7) {
        a.push(v(&["f1"]));
    } else if g.bool_with(0.3) {
        a.push(v(&["-"]));
    }
    mk_keep("ptx", pin, a, stdin, vec![("f1".into(), f1), ("f2".into(), f2)])
}

fn tsort(g: &mut Source) -> Case {
    let toks = ["a", "b", "c", "d", "e", "f", "a1", "-"];
    let n = g.int(0..=10usize);
    let mut s = String::new();
    for i in 0..n {
        s.push_str(toks[g.index(toks.len())]);
        s.push_str(if i + 1 == n { "" } else { pick(g, &[" ", " ", "\n", "\t", "  "]) });
    }
    if g.bool_with(0.7) {
        s.push('\n');
    }
    let mut a = Vec::new();
    match *g.weighted(&[(6, 0), (3, 1), (1, 2), (1, 3)]) {
        0 => {}
        1 => a.push(v(&["f1"])),
        2 => a.push(v(&["-"])),
        _ => a.push(v(&["f1", "f2"])),
    }
    mk("tsort", a, s.clone().into_bytes(), vec![("f1".into(), s.into_bytes()), ("f2".into(), b"x y\n".to_vec())])
}

// ---------------------------------------------------------------- env -S

fn env_split_string(g: &mut Source) -> String {
    let mut s = String::new();
    if g.bool_with(0.4) {
        s.push_str(pick(g, &["A=1 ", "B=x\\_y ", "A=\"q r\" ", "C= ", "A=1 B=2 "]));
    }
    s.push_str(pick(g, &["printenv", "echo", "printenv A", "echo -n", "printf %s|"]));
    for _ in 0..g.int(0..=4usize) {
        s.push_str(pick(g, &[" ", "  ", "\\_", "\t", "\\t"]));
        s.push_str(pick(
            g,
            &[
                "a", "'s q'", "\"d q\"", "\"\\t\\n\"", "'\\t'", "${A}", "$A", "\"${B}x\"", "\\c", "#c", "x#y", "\\\\", "\\$",
                "\\#", "'a\"b'", "\"a'b\"", "\\v\\f\\r", "\"\"", "''", "${UNSET}", "\\\"", "${A", "$", "\\q", "a\\ b",
            ],
        ));
    }
    s
}

fn env(g: &mut Source) -> Case {
    let pin: Vec<Vec<String>> = vec![v(&["-i"])];
    let mut a: Vec<Vec<String>> = Vec::new();
    let mut assigns: Vec<Vec<String>> = Vec::new();
    // printing mode: GNU 9.12 quotes values that need it (A='x y'), uutils prints
    // them raw; values here are shell-safe. With -i the command comes from the
    // default PATH on both sides.
    let safe = ["A=1", "B=", "C=xy", "A=2", "D_1=/a/b", "E=-"];
    for _ in 0..*g.weighted(&[(3, 0usize), (3, 1), (1, 2)]) {
        assigns.push(one(pick(g, &safe).into()));
    }
    if g.bool_with(0.2) {
        // GNU ignores -u '' and -u '=' (exit 0), uutils rejects them: rare
        a.push(vec!["-u".into(), if g.bool_with(0.1) { pick(g, &["=", ""]) } else { pick(g, &["A", "B", "Q"]) }.into()]);
    }
    // the synopsis puts options before NAME=VALUE (GNU takes a later option
    // for a NAME=VALUE or the command); rarely the other order
    if g.bool_with(0.03) {
        a.append(&mut assigns);
    }
    match *g.weighted(&[(5, 0), (2, 1), (2, 2), (1, 3)]) {
        0 => {
            let s = env_split_string(g);
            a.push(if g.bool_with(0.5) { vec!["-S".into(), s] } else { one(format!("--split-string={s}")) });
            a.append(&mut assigns);
            for _ in 0..g.int(0..=2usize) {
                a.push(one(pick(g, &["z", "a b", "$A", "\\t"]).into()));
            }
        }
        1 => {
            if g.bool_with(0.3) {
                a.push(v(&["-0"]));
            }
            a.append(&mut assigns);
        }
        2 => {
            a.append(&mut assigns);
            a.push(one(pick(g, &["printenv", "true", "false", "nocmd", "echo"]).into()));
            for _ in 0..g.int(0..=2usize) {
                a.push(one(pick(g, &["A", "B", "-0", "x"]).into()));
            }
        }
        _ => {
            a.push(vec!["-a".into(), pick(g, &["foo", ""]).into()]);
            a.append(&mut assigns);
            a.push(v(&["printenv"]));
        }
    }
    mk_keep("env", pin, a, vec![], vec![])
}

// ---------------------------------------------------------------- file trees

/// A small tree: files, nested directories, symlinks (one dangling).
fn tree_files(g: &mut Source) -> Vec<(String, Vec<u8>)> {
    let mut f: Vec<(String, Vec<u8>)> = vec![("f1".into(), text(g)), ("f2".into(), text(g))];
    if g.bool_with(0.8) {
        f.push(("d1/".into(), vec![]));
        f.push(("d1/a".into(), b"a\n".to_vec()));
        if g.bool_with(0.6) {
            f.push(("d1/sub/".into(), vec![]));
            f.push(("d1/sub/b".into(), b"bb\n".to_vec()));
        }
        if g.bool_with(0.3) {
            f.push(("d1/f1".into(), b"old\n".to_vec()));
        }
    }
    if g.bool_with(0.4) {
        f.push(("d2/".into(), vec![]));
    }
    if g.bool_with(0.5) {
        f.push(("l1@".into(), b"f1".to_vec()));
    }
    if g.bool_with(0.3) {
        f.push(("l2@".into(), b"d1".to_vec()));
    }
    if g.bool_with(0.2) {
        f.push(("l3@".into(), b"nofile".to_vec()));
    }
    if g.bool_with(0.15) {
        f.push(("f1~".into(), b"backup\n".to_vec()));
    }
    if g.bool_with(0.1) {
        f.push(("f2.~1~".into(), b"numbered\n".to_vec()));
    }
    f
}

fn fpath(g: &mut Source) -> String {
    pick(
        g,
        &[
            "f1", "f1", "f2", "f2", "d1", "d1", "d2", "l1", "l2", "l3", "f3", "d3", "d1/", "d1/a", "d1/sub", "d1/sub/b", "l2/",
            "d3/x", "d1/f1", ".", "f1/", "d2/", "l1/",
        ],
    )
    .into()
}

fn fs_operands(g: &mut Source, lo: usize, hi: usize) -> Vec<Vec<String>> {
    let n = g.int(lo..=hi);
    let mut out: Vec<Vec<String>> = Vec::new();
    for _ in 0..n {
        let p = fpath(g);
        if !out.iter().any(|o| o[0] == p) {
            out.push(one(p));
        }
    }
    out
}

fn sym_mode(g: &mut Source) -> String {
    let n = *g.weighted(&[(5, 1usize), (2, 2), (1, 3)]);
    (0..n)
        .map(|_| {
            let who: String = (0..g.int(0..=2usize)).map(|_| pick(g, &["u", "g", "o", "a"])).collect();
            let op = pick(g, &["+", "-", "="]);
            let perm: String = if g.bool_with(0.15) {
                pick(g, &["u", "g", "o"]).into()
            } else {
                (0..g.int(0..=3usize)).map(|_| pick(g, &["r", "w", "x", "X", "s", "t"])).collect()
            };
            let mut s = format!("{who}{op}{perm}");
            if g.bool_with(0.1) {
                s.push_str(&format!("{}{}", pick(g, &["+", "-", "="]), pick(g, &["r", "x", "w"])));
            }
            s
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn mode_arg(g: &mut Source) -> String {
    match *g.weighted(&[(4, 0), (5, 1), (1, 2)]) {
        0 => pick(g, &["755", "644", "0", "700", "1777", "2755", "4711", "0644", "00755", "640", "7777", "511", "8", "-1", "07000"]).into(),
        1 => sym_mode(g),
        _ => pick(g, &["", "u", "+", "u=rw,", ",", "a=", "ugo+rwx", "=", "u+z", "755,u+w"]).into(),
    }
}

fn backup(g: &mut Source) -> Vec<String> {
    match g.int(0..=4u32) {
        0 => v(&["-b"]),
        1 => one(format!("--backup={}", pick(g, &["simple", "numbered", "existing", "none", "t", "nil", "never", "off", "bogus"]))),
        2 => v(&["--backup"]),
        3 => vec!["-S".into(), pick(g, &[".bak", "~", "", "/x"]).into()],
        _ => one(format!("--suffix={}", pick(g, &[".s", "_"]))),
    }
}

fn chmod(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| one(format!("-{}", pick(g, &["R", "c", "v", "f"]))),
        |g| one(pick(g, &["--preserve-root", "--no-preserve-root", "-h", "--dereference", "-H", "-L", "-P"]).into()),
        |g| one(format!("--reference={}", pick(g, &["f2", "d1", "nofile", "l1"]))),
    ];
    let mut a = opts(g, 3, T);
    if !a.iter().any(|o| o[0].starts_with("--reference")) {
        a.push(one(mode_arg(g)));
    }
    // chmod on "." changes the access to every later operand. With -v/-c, GNU
    // reports a missing operand on stdout too ("could not be accessed"): mostly
    // existing operands then.
    let verbose = a.iter().any(|o| o[0] == "-v" || o[0] == "-c") && !g.bool_with(0.15);
    let exists = |p: &str| matches!(p, "f1" | "f2" | "d1" | "l1" | "d1/a");
    a.extend(fs_operands(g, 1, 3).into_iter().filter(|o| o[0] != "." && (!verbose || exists(&o[0]))));
    if a.last().map_or(true, |o| o[0].starts_with('-') || o[0].starts_with("--")) {
        a.push(v(&["f1"]));
    }
    mk("chmod", a, vec![], tree_files(g))
}

fn mkdir(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| one(pick(g, &["-p", "-v", "--parents"]).into()),
        // set-id and sticky bits in mkdir -m end up differently on the new
        // directory in uutils: one class, mostly left out
        |g| vec!["-m".into(), if g.bool_with(0.2) { mode_arg(g) } else { mode_arg(g).replace(['s', 't'], "") }],
    ];
    let mut a = opts(g, 3, T);
    for _ in 0..g.int(1..=2usize) {
        a.push(one(pick(g, &["d3", "d3/x/y", "d1", "f1/x", "d1/sub", "d4/", "l2/new", "d3/../d5", "d1/sub/c", "l3", "d6//e"]).into()));
    }
    mk("mkdir", a, vec![], tree_files(g))
}

fn rmdir(g: &mut Source) -> Case {
    const T: &[OptFn] = &[|g| one(pick(g, &["-p", "-v", "--ignore-fail-on-non-empty", "--parents"]).into())];
    let mut a = opts(g, 3, T);
    for _ in 0..g.int(1..=2usize) {
        a.push(one(pick(g, &["d2", "d1", "d1/sub", "f1", "l2", "d2/", "nofile", "l2/", "d1/sub/", "d2/."]).into()));
    }
    mk("rmdir", a, vec![], tree_files(g))
}

/// rm/cp/mv -i read their answers from stdin.
fn answers(g: &mut Source) -> Vec<u8> {
    pick(g, &["y\n", "n\n", "y\ny\ny\n", "yes\nno\n", "", "Y\n", "n\ny\n"]).as_bytes().to_vec()
}

fn rm(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| one(format!("-{}", pick(g, &["R", "d", "i", "I"]))),
        |g| one(format!("-{}", pick(g, &["r", "f", "v"]))),
        |g| one(pick(g, &["--one-file-system", "--preserve-root", "--interactive=never", "--interactive=once", "--dir"]).into()),
    ];
    let mut a = opts(g, 3, T);
    a.extend(fs_operands(g, 1, 3));
    mk("rm", a, answers(g), tree_files(g))
}

fn ln(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| one(format!("-{}", pick(g, &["s", "s", "f", "n", "T", "r", "v", "L", "P", "i"]))),
        |_| v(&["-s"]),
        |g| backup(g),
        |g| vec!["-t".into(), pick(g, &["d1", "d2", "l2", "f1", "nodir"]).into()],
    ];
    let mut a = opts(g, 3, T);
    a.extend(fs_operands(g, 1, 3));
    mk("ln", a, answers(g), tree_files(g))
}

fn cp(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| one(format!("-{}", pick(g, &["R", "f", "n", "l", "s", "L", "P", "H", "T", "v", "d", "x", "i", "p"]))),
        |g| one(format!("-{}", pick(g, &["r", "a"]))),
        |g| {
            one(pick(
                g,
                &[
                    "--parents", "--remove-destination", "--no-dereference", "--preserve=mode", "--no-preserve=mode",
                    "--attributes-only", "--strip-trailing-slashes", "--update=none", "--update=all", "--update=none-fail",
                    "--debug", "--copy-contents", "--sparse=always", "--reflink=never", "--keep-directory-symlink",
                ],
            )
            .into())
        },
        |g| backup(g),
        |g| vec!["-t".into(), pick(g, &["d1", "d2", "l2", "f1", "nodir"]).into()],
    ];
    let mut a = opts(g, 4, T);
    a.extend(fs_operands(g, 1, 3));
    mk("cp", a, answers(g), tree_files(g))
}

fn mv(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| one(format!("-{}", pick(g, &["f", "n", "T", "v", "i"]))),
        |g| one(pick(g, &["--strip-trailing-slashes", "--update=none", "--update=all", "--exchange", "--no-copy", "--debug"]).into()),
        |g| backup(g),
        |g| vec!["-t".into(), pick(g, &["d1", "d2", "l2", "f1", "nodir"]).into()],
    ];
    let mut a = opts(g, 3, T);
    a.extend(fs_operands(g, 1, 3));
    mk("mv", a, answers(g), tree_files(g))
}

fn install(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| one(format!("-{}", pick(g, &["d", "D", "v", "C", "T", "p"]))),
        |g| vec!["-m".into(), mode_arg(g)],
        |g| backup(g),
        |g| vec!["-t".into(), pick(g, &["d1", "d2", "d3", "f1"]).into()],
    ];
    let mut a = opts(g, 3, T);
    a.extend(fs_operands(g, 1, 3));
    mk("install", a, vec![], tree_files(g))
}

fn stat(g: &mut Source) -> Case {
    // no times, inodes or device numbers: they differ between the two trees
    let dirs = ["a", "A", "b", "B", "f", "F", "g", "G", "h", "n", "N", "s", "u", "U", "t", "T"];
    let mut f = String::new();
    for _ in 0..g.int(1..=4usize) {
        if g.bool_with(0.15) {
            f.push_str(pick(g, &[" ", "x", "\\n", "\\t", "\\0", "\\x41", "\\101", "\\", "\\q"]));
            continue;
        }
        // a flag or width before '%' is an invalid directive for GNU, uutils
        // prints '?': rare
        if g.bool_with(0.05) {
            f.push_str("%%");
            continue;
        }
        f.push('%');
        if g.bool_with(0.15) {
            f.push_str(pick(g, &["-", "0", "#", "+", " ", "'"]));
        }
        if g.bool_with(0.15) {
            f.push_str(&num(g, 0, 8));
        }
        if g.bool_with(0.05) {
            f.push_str(&format!(".{}", num(g, 0, 3)));
        }
        f.push_str(pick(g, &dirs));
    }
    let mut a: Vec<Vec<String>> = Vec::new();
    let pin = vec![match g.int(0..=2u32) {
        0 => vec!["-c".into(), f],
        1 => one(format!("--printf={f}")),
        _ => one(format!("--format={f}")),
    }];
    if g.bool_with(0.3) {
        a.push(v(&["-L"]));
    }
    a.extend(fs_operands(g, 1, 3));
    mk_keep("stat", pin, a, vec![], tree_files(g))
}

/// ls: names that sort, quote and classify differently.
fn ls_files(g: &mut Source) -> Vec<(String, Vec<u8>)> {
    let mut f = tree_files(g);
    let names = ["a b", "x.txt", "x.tar.gz", "file10", "file9", "file1.2", "UPPER", ".hidden", "'q", "tab\tx", "e\\s", "-dash", "z~", "Ab", "a"];
    for _ in 0..g.int(0..=5usize) {
        let n = names[g.index(names.len())];
        if !f.iter().any(|(x, _)| x == n) {
            let size = g.int(0..=3000usize);
            f.push((n.to_string(), vec![b'x'; size]));
        }
    }
    f
}

fn ls(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| one(format!("-{}", pick(g, &["a", "A", "1", "m", "F", "p", "R", "r", "S", "X", "v", "U", "d", "L", "H", "G", "h", "s", "Q", "b", "N", "q", "k", "B"]))),
        // grids (-C, -x): GNU pads with tabs to 8-column stops and sizes the
        // columns differently; one class, so rarer than the rest
        |g| if g.bool_with(0.3) { one(pick(g, &["-C", "-x", "--format=across", "--format=vertical", "--format=horizontal"]).into()) } else { v(&["-1"]) },
        |g| one(format!("-{}", pick(g, &["l", "g", "o", "n", "l"]))),
        |g| one(format!("--quoting-style={}", pick(g, &["literal", "shell", "shell-always", "shell-escape", "shell-escape-always", "c", "escape", "locale", "clocale"]))),
        |g| one(format!("--indicator-style={}", pick(g, &["none", "slash", "file-type", "classify"]))),
        |g| one(format!("--sort={}", pick(g, &["none", "size", "version", "extension", "name", "width"]))),
        |g| one(format!("--format={}", pick(g, &["commas", "long", "single-column", "verbose"]))),
        |g| vec!["-w".into(), num(g, 0, 40)],
        |g| vec!["-T".into(), num(g, 0, 8)],
        |g| vec!["-I".into(), pick(g, &["*.txt", "f*", "d?", "*"]).into()],
        |g| one(format!("--hide={}", pick(g, &["*.txt", "f*", "x*"]))),
        |g| one(format!("--block-size={}", if g.bool_with(0.1) { pick(g, &["K", "MB", "'1"]) } else { pick(g, &["1", "1K", "512", "2"]) })),
        |g| one(pick(g, &["--group-directories-first", "--si", "--zero", "--dereference-command-line-symlink-to-dir", "--classify=always", "--file-type", "--literal", "--show-control-chars", "--hyperlink=never", "--color=never"]).into()),
    ];
    let mut a = opts(g, 4, T);
    // long formats show times: a fixed style
    if g.bool_with(0.5) {
        a.extend(fs_operands(g, 1, 3));
    }
    mk_keep("ls", vec![v(&["--time-style=+T"])], a, vec![], ls_files(g))
}

fn du(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| one(format!("-{}", pick(g, &["a", "b", "s", "c", "h", "k", "m", "L", "P", "S", "x", "l", "0", "D", "H"]))),
        |g| one(pick(g, &["--apparent-size", "--si", "--inodes", "--null", "--count-links"]).into()),
        |g| vec!["-d".into(), num(g, 0, 3)],
        |g| one(format!("--max-depth={}", num(g, 0, 2))),
        // a unit without a number (-B K) is printed after each size by GNU only: rare
        |g| vec!["-B".into(), if g.bool_with(0.1) { pick(g, &["K", "MB", "0", "x"]) } else { pick(g, &["1", "1K", "512", "2", "1M"]) }.into()],
        |g| one(format!("--exclude={}", pick(g, &["*.txt", "a", "sub", "d*", "f?"]))),
        |g| vec!["-t".into(), pick(g, &["1", "-4096", "5k", "0", "8192"]).into()],
    ];
    let mut a = opts(g, 4, T);
    // block usage depends on the filesystem's allocation; apparent sizes do not
    a.retain(|o| o[0] != "-b" && o[0] != "--apparent-size" && o[0] != "--inodes");
    let pin = vec![v(&[pick(g, &["--apparent-size", "-b", "--inodes"])])];
    if g.bool_with(0.7) {
        a.extend(fs_operands(g, 1, 3));
    }
    mk_keep("du", pin, a, vec![], ls_files(g))
}

fn realpath(g: &mut Source) -> Case {
    const T: &[OptFn] = &[
        |g| one(format!("-{}", pick(g, &["e", "m", "s", "L", "P", "q", "z"]))),
        // paths outside the base are printed absolute, and differ between the two sides
        |_| one("--relative-base=.".into()),
    ];
    let mut a = opts(g, 3, T);
    // absolute paths differ between the two working directories
    let pin = vec![one(format!("--relative-to={}", pick(g, &[".", ".", "d1", "d1/sub", "l2", "nodir", "f1"])))];
    a.extend(fs_operands(g, 1, 3));
    mk_keep("realpath", pin, a, vec![], tree_files(g))
}

fn readlink(g: &mut Source) -> Case {
    const T: &[OptFn] = &[|g| one(format!("-{}", pick(g, &["n", "q", "s", "v", "z"])))];
    let mut a = opts(g, 2, T);
    a.extend(fs_operands(g, 1, 3));
    mk("readlink", a, vec![], tree_files(g))
}

// ---------------------------------------------------------------- top level

pub const UTILS: &[(&str, u32, fn(&mut Source) -> Case)] = &[
    ("sort", 16, sort),
    ("uniq", 10, uniq),
    ("cut", 12, cut),
    ("tr", 1, tr),
    ("head", 8, head),
    ("tail", 10, tail),
    ("wc", 6, wc),
    ("nl", 4, nl),
    ("fold", 4, fold),
    ("expand", 1, expand),
    ("unexpand", 1, unexpand),
    ("paste", 6, paste),
    ("comm", 1, comm),
    ("join", 1, join),
    ("tac", 6, tac),
    ("od", 10, od),
    ("numfmt", 8, numfmt),
    ("seq", 6, seq),
    ("printf", 8, printf),
    ("expr", 6, expr),
    ("test", 6, test),
    ("basename", 2, basename),
    ("dirname", 2, dirname),
    ("base64", 2, base64),
    ("base32", 2, base32),
    ("basenc", 6, basenc),
    ("cksum", 6, cksum),
    ("factor", 6, factor),
    ("split", 6, split),
    ("csplit", 6, csplit),
    ("truncate", 6, truncate),
    ("cat", 2, cat),
    ("echo", 2, echo),
    ("fmt", 1, fmt),
    ("dd", 4, dd),
    ("date", 3, date),
    ("pr", 1, pr),
    ("ptx", 1, ptx),
    ("tsort", 1, tsort),
    ("env", 2, env),
    ("chmod", 2, chmod),
    ("mkdir", 1, mkdir),
    ("rmdir", 1, rmdir),
    ("rm", 2, rm),
    ("ln", 2, ln),
    ("cp", 3, cp),
    ("mv", 2, mv),
    ("install", 1, install),
    ("stat", 2, stat),
    ("ls", 2, ls),
    ("du", 1, du),
    ("realpath", 1, realpath),
    ("readlink", 1, readlink),
];

pub fn case(g: &mut Source, only: Option<&[String]>) -> Case {
    let table: Vec<(u32, usize)> = UTILS
        .iter()
        .enumerate()
        .filter(|(_, u)| only.map_or(true, |o| o.iter().any(|x| x == u.0)))
        .map(|(i, u)| (u.1, i))
        .collect();
    let i = *g.weighted(&table);
    (UTILS[i].2)(g)
}

// ---------------------------------------------------------------- shrinking

fn shrink_bytes(b: &[u8], visit: &mut dyn FnMut(Vec<u8>) -> ControlFlow<()>) -> ControlFlow<()> {
    if b.is_empty() {
        return ControlFlow::Continue(());
    }
    visit(Vec::new())?;
    // drop one line
    let lines: Vec<&[u8]> = b.split_inclusive(|&c| c == b'\n').collect();
    if lines.len() > 1 {
        for i in 0..lines.len() {
            let mut x = Vec::new();
            for (j, l) in lines.iter().enumerate() {
                if j != i {
                    x.extend_from_slice(l);
                }
            }
            visit(x)?;
        }
    }
    // drop one byte (bounded)
    for i in 0..b.len().min(64) {
        let mut x = b.to_vec();
        x.remove(i);
        visit(x)?;
    }
    ControlFlow::Continue(())
}

pub fn shrink_case(c: &Case, visit: &mut dyn FnMut(Case) -> ControlFlow<()>) -> ControlFlow<()> {
    for i in c.keep..c.args.len() {
        let mut x = c.clone();
        x.args.remove(i);
        visit(x)?;
    }
    shrink_bytes(&c.stdin, &mut |b| {
        let mut x = c.clone();
        x.stdin = b;
        visit(x)
    })?;
    for i in 0..c.files.len() {
        if c.files[i].0.ends_with('@') {
            continue; // symlink target
        }
        shrink_bytes(&c.files[i].1, &mut |b| {
            let mut x = c.clone();
            x.files[i].1 = b;
            visit(x)
        })?;
    }
    ControlFlow::Continue(())
}
