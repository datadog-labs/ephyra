// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Script generator: a tree of statements (so the shrinker can delete subtrees) rendered
//! to bash text. Valid by construction: every construct here parses in bash 5.3.
//! Small domains (3 scalars, 2 arrays, 1 integer var, 1 assoc, ~6 files) so values collide.

use ephyra::Source;

/// One statement: a list of segments. `Body` segments are statement lists the shrinker
/// may delete from; an empty body renders as `:` so the result still parses.
#[derive(Clone, Debug)]
pub enum Seg {
    Text(String),
    Body(Vec<Node>),
}

#[derive(Clone, Debug)]
pub struct Node(pub Vec<Seg>);

impl Node {
    fn leaf(s: impl Into<String>) -> Node {
        Node(vec![Seg::Text(s.into())])
    }
}

/// Always enabled: brush appears to honour extglob patterns whether or not the option is
/// set, and bash's parser rejects them when it is off (see notes), so keep it on.
pub const PRELUDE: &str = "shopt -s extglob\nmkdir sub; : > f1; echo hi > f2.txt; : > g.log; : > .h; : > 'd e'; : > sub/x.txt\n";

pub fn render(nodes: &[Node]) -> String {
    let mut out = String::from(PRELUDE);
    render_into(nodes, &mut out);
    out
}

fn render_into(nodes: &[Node], out: &mut String) {
    for n in nodes {
        for s in &n.0 {
            match s {
                Seg::Text(t) => {
                    out.push_str(t);
                    out.push('\n');
                }
                Seg::Body(b) => {
                    if b.is_empty() {
                        out.push_str(":\n");
                    } else {
                        render_into(b, out);
                    }
                }
            }
        }
    }
}

pub fn count(nodes: &[Node]) -> usize {
    nodes
        .iter()
        .map(|n| {
            1 + n
                .0
                .iter()
                .map(|s| if let Seg::Body(b) = s { count(b) } else { 0 })
                .sum::<usize>()
        })
        .sum()
}

/// Delete the k-th node in pre-order. Returns None if k is out of range.
pub fn delete_nth(nodes: &[Node], k: usize) -> Option<Vec<Node>> {
    let mut k = k;
    del(nodes, &mut k)
}

fn del(nodes: &[Node], k: &mut usize) -> Option<Vec<Node>> {
    for (i, n) in nodes.iter().enumerate() {
        if *k == 0 {
            let mut v = nodes.to_vec();
            v.remove(i);
            return Some(v);
        }
        *k -= 1;
        for (j, s) in n.0.iter().enumerate() {
            if let Seg::Body(b) = s {
                if let Some(nb) = del(b, k) {
                    let mut v = nodes.to_vec();
                    v[i].0[j] = Seg::Body(nb);
                    return Some(v);
                }
            }
        }
    }
    None
}

/// Replace the k-th node (pre-order) by the contents of its first body, if it has one
/// (unwraps `if`/`for`/function wrappers).
pub fn hoist_nth(nodes: &[Node], k: usize) -> Option<Vec<Node>> {
    let mut k = k;
    hoist(nodes, &mut k).flatten()
}

/// Outer None: k not in this subtree. Some(None): found, but it has no body.
fn hoist(nodes: &[Node], k: &mut usize) -> Option<Option<Vec<Node>>> {
    for (i, n) in nodes.iter().enumerate() {
        if *k == 0 {
            let is_loop = matches!(n.0.first(), Some(Seg::Text(t)) if t.starts_with("for "));
            let Some(body) = n.0.iter().filter(|_| !is_loop).find_map(|s| if let Seg::Body(b) = s { Some(b.clone()) } else { None }) else {
                return Some(None);
            };
            let mut v = nodes[..i].to_vec();
            v.extend(body);
            v.extend_from_slice(&nodes[i + 1..]);
            return Some(Some(v));
        }
        *k -= 1;
        for (j, s) in n.0.iter().enumerate() {
            if let Seg::Body(b) = s {
                match hoist(b, k) {
                    None => {}
                    Some(None) => return Some(None),
                    Some(Some(nb)) => {
                        let mut v = nodes.to_vec();
                        v[i].0[j] = Seg::Body(nb);
                        return Some(Some(v));
                    }
                }
            }
        }
    }
    None
}

const SCALARS: &[&str] = &["v", "w", "x"];
const ARRAYS: &[&str] = &["a", "b"];
const FILES: &[&str] = &["f1", "f2.txt", "g.log", ".h", "d e", "sub/x.txt"];

/// String values, chosen to exercise splitting, globbing, patterns and case conversion.
const VALUES: &[&str] = &[
    "", "a", "b c", " x  y ", "a*", "*", "1", "-3", "10", "a:b:c", "x/y/z", "ab.cd.ef", "AbC dEf",
    "'", "\"", "?", "[ab]", "-n", "-e", "foo bar baz", "aaa", "f*", "*.txt", "0x1f", "a\tb", "a\nb",
    "::", ":a:", "%", "$", "ab", "b", "bab", "a-b", "3 4",
];

const PATTERNS: &[&str] = &[
    "a", "*", "?", "a*", "*a", "b*", "[ab]", "[!a]*", "*.", ".*", "[[:upper:]]", "[[:space:]]", ":", "*:",
    "\\*", "'*'", "x/", "+(a)", "@(ab|b)", "[a-c]",
];

pub struct G<'a> {
    pub s: &'a mut Source,
    funcs: Vec<String>,
    /// functions whose body hits a known printer bug of `declare -f` (multi-line quoted text, a
    /// multi-line `$( )` or a here-doc gets indented, a compound's redirection prints as `}2>`): their round-trips are kept rare
    fragile: Vec<String>,
    in_func: bool,
    /// name of the function whose body is being generated: a body only calls functions whose
    /// name sorts before it, so no call cycle (self or mutual recursion) can be generated.
    /// Unbounded recursion ends on fd/stack exhaustion, whose depth differs between shells
    /// (brush starts with more fds open), not on anything the shells specify.
    cur_func: Option<String>,
    loop_depth: usize,
    in_cmdsub: bool,
    in_heredoc: bool,
    /// no "$@"/"${a[@]}"-like parts: in a multi-part quoted word an empty [@] expansion next
    /// to empty text gives 0 words in bash, 1 in brush (notes)
    no_at: bool,
    /// no side effects in arithmetic (inside words, bash evaluates lazily or in subshells)
    pure: bool,
    /// inside `{ ... } > f`: stdout may be a file the script also touches
    redir_depth: usize,
}

/// A redirection right after a compound command (`} >f`, `]] 2>/dev/null`, `)) >f`): `declare -f`
/// prints it without the separating space (`}2> /dev/null`), which re-reads as a different command.
pub fn compound_redir(text: &str) -> bool {
    ["]]", "}", "))", ")"].iter().any(|c| {
        text.match_indices(c).any(|(i, _)| {
            let rest = text[i + c.len()..].trim_start_matches(' ');
            let rest = rest.strip_prefix(|d: char| d.is_ascii_digit()).unwrap_or(rest);
            rest.starts_with('>') || rest.starts_with('<')
        })
    })
}

pub fn sq(s: &str) -> String {
    if s == "'" {
        // not ''\''': bash 5.3 fails to parse that inside $( ) (notes)
        return r#""'""#.into();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

impl<'a> G<'a> {
    pub fn new(s: &'a mut Source) -> Self {
        G { s, funcs: vec![], fragile: vec![], in_func: false, cur_func: None, loop_depth: 0, in_cmdsub: false, in_heredoc: false, no_at: false, pure: true, redir_depth: 0 }
    }

    fn pick(&mut self, xs: &[&str]) -> String {
        self.s.pick(xs).to_string()
    }
    fn p(&mut self, prob: f64) -> bool {
        self.s.bool_with(prob)
    }
    fn n(&mut self, lo: i64, hi: i64) -> i64 {
        self.s.int(lo..=hi)
    }

    // ---------- words ----------

    fn value_lit(&mut self) -> String {
        let v = self.pick(VALUES);
        sq(&v)
    }

    fn pattern(&mut self) -> String {
        // patterns are rendered unquoted (so they act as patterns) unless they contain
        // characters that would break the parse.
        let p = self.pick(PATTERNS);
        if self.p(0.15) {
            format!("\"{}\"", self.param_simple())
        } else {
            p
        }
    }

    /// Pattern for ${v/pat/rep}: never empty (brush substitutes on an empty pattern, bash
    /// does not: see notes).
    fn subpat(&mut self) -> String {
        // not `*` and not a (possibly empty) quoted parameter: on an empty value / empty
        // pattern bash substitutes nothing, brush substitutes (notes)
        loop {
            let p = self.pick(PATTERNS);
            if p != "*" {
                return p;
            }
        }
    }

    /// Pattern for the shortest-match ${v#pat} / ${v%pat}: not `*` (brush removes one
    /// character where bash removes the empty match: see notes).
    fn shortpat(&mut self) -> String {
        loop {
            let p = self.pattern();
            if p != "*" {
                return p;
            }
        }
    }

    fn param_simple(&mut self) -> String {
        let v = self.pick(SCALARS);
        format!("${{{v}}}")
    }

    fn arith_atom(&mut self) -> String {
        match self.n(0, 9) {
            0 => self.n(-3, 10).to_string(),
            1 => self.pick(&["0", "1", "2", "7", "-1", "64", "255"]),
            2 => self.pick(&["0x1f", "017", "2#101", "16#ff", "36#zz", "8#7", "010", "0X10", "64#_"]),
            3 if self.s.bool_with(0.3) => self.pick(&["9223372036854775807", "-9223372036854775808", "4294967296"]),
            // only variables that hold integers: an arithmetic error inside (( )) ends the
            // script in brush but not in bash (notes), so expressions are kept error-free
            4 | 5 => self.pick(&["n", "i", "${#v}", "${#a[@]}"]),
            6 if !self.pure => self.pick(&["n++", "n--", "++n", "--n", "(n+=2)", "(i=3)", "(n*=2)"]),
            7 => format!("${{#{}}}", self.pick(SCALARS)),
            _ => self.n(0, 5).to_string(),
        }
    }

    pub fn arith(&mut self, d: usize) -> String {
        if d == 0 || self.p(0.35) {
            return self.arith_atom();
        }
        match self.n(0, 9) {
            0..=4 => {
                let op = self.pick(&[
                    "+", "-", "*", "/", "%", "**", "<<", ">>", "&", "|", "^", "&&", "||", "==", "!=", "<", "<=",
                    ">", ">=", ",",
                ]);
                let rhs = match op.as_str() {
                    "/" | "%" => format!("(({}) | 1)", self.arith(d - 1)),
                    "**" => format!("(({}) & 3)", self.arith(d - 1)),
                    _ => self.arith(d - 1),
                };
                format!("{} {} {}", self.arith(d - 1), op, rhs)
            }
            5 => format!("({})", self.arith(d - 1)),
            6 => format!("{} {}", self.pick(&["-", "!", "~", "+"]), self.arith(d - 1)),
            7 => format!("{} ? {} : {}", self.arith(d - 1), self.arith(d - 1), self.arith(d - 1)),
            8 if !self.pure => format!("({} {} ({}))", self.pick(&["n", "i"]), self.pick(&["=", "+=", "-=", "*=", "<<=", "|="]), self.arith(d - 1)),
            _ => format!("({})", self.arith(d - 1)),
        }
    }

    /// A word that is a parameter expansion (always braced).
    fn param_exp(&mut self, d: usize) -> String {
        let v = self.pick(SCALARS);
        // replacement/default words: unquoted plain text or a nested expansion (quotes inside
        // double-quoted ${..} replacements are a known brush difference, issue #682)
        let sub = |me: &mut Self| if d > 0 && me.p(0.3) { me.param_exp(d - 1) } else { me.pick(&["a", "X", "_", "1", "ab", "", "z"]) };
        match self.n(0, 21) {
            0 => format!("${v}"),
            1 => format!("${{{v}}}"),
            2 => format!("${{#{v}}}"),
            3 => format!("${{{v}#{}}}", self.shortpat()),
            4 => format!("${{{v}##{}}}", self.pattern()),
            5 => format!("${{{v}%{}}}", self.shortpat()),
            6 => format!("${{{v}%%{}}}", self.pattern()),
            7 => format!("${{{v}/{}/{}}}", self.subpat(), sub(self)),
            8 => format!("${{{v}//{}/{}}}", self.subpat(), sub(self)),
            9 => format!("${{{v}/#{}/{}}}", self.subpat(), sub(self)),
            10 => format!("${{{v}/%{}/{}}}", self.subpat(), sub(self)),
            11 => format!("${{{v}{}}}", self.pick(&["^", "^^", ",", ",,"])),
            12 => format!("${{{v}:{}}}", self.offset()),
            13 => format!("${{{v}:{}:{}}}", self.offset(), self.length()),
            // bash expands here-documents without side effects on the shell (notes)
            14 if self.in_heredoc => format!("${{{v}{}{}}}", self.pick(&[":-", "-", ":+", "+"]), sub(self)),
            14 => format!("${{{v}{}{}}}", self.pick(&[":-", "-", ":+", "+", ":=", "="]), sub(self)),
            15 => format!("${{{v}@{}}}", self.pick(&["U", "L", "a", "E"])),
            16 => format!("${{{v}^^{}}}", self.pick(&["a", "[a-c]", "?"])),
            17 => format!("${{{v}:-}}"),
            18 => format!("${{{v}//{}}}", self.subpat()),
            19 => format!("${{#{v}}}"),
            20 => format!("${{{v},,{}}}", self.pick(&["A", "[A-C]", "?"])),
            _ => format!("${v}"),
        }
    }

    /// Substring offset: small, may be negative.
    fn offset(&mut self) -> String {
        match self.n(0, 4) {
            0 => self.n(0, 4).to_string(),
            1 => format!(" -{}", self.n(1, 4)),
            2 => format!("(({}) & 7)", self.arith(1)),
            3 => "n".into(),
            _ => self.n(-1, 6).to_string().replace('-', " -"),
        }
    }

    /// Substring length: small, non-negative (a negative end before the offset is an
    /// expansion error, after which brush abandons more of the script than bash: notes).
    fn length(&mut self) -> String {
        match self.n(0, 2) {
            0 => self.n(0, 4).to_string(),
            1 => format!("(({}) & 7)", self.arith(1)),
            _ => "${#w}".into(),
        }
    }

    fn array_exp(&mut self) -> String {
        let a = self.pick(ARRAYS);
        match self.n(0, 15) {
            0 => format!("${{{a}[@]}}"),
            1 => format!("${{{a}[*]}}"),
            2 => format!("${{#{a}[@]}}"),
            3 => format!("${{!{a}[@]}}"),
            4 => format!("${{{a}[({}) & 3]}}", self.arith(1)),
            // small non-negative lengths: a negative length is an expansion error for arrays,
            // and empty slices next to empty text hit a known word-removal difference (notes)
            5 => format!("${{{a}[@]:{}}}", self.pick(&["0", "1", " -1", "n&1"])),
            6 => format!("${{{a}[@]:{}:{}}}", self.pick(&["0", "1", " -1", "n&1"]), self.pick(&["1", "2", "0"])),
            7 => format!("${{{a}[@]/{}/{}}}", self.subpat(), self.value_lit_or_plain()),
            8 => format!("${{{a}[@]#{}}}", self.shortpat()),
            9 => format!("${{{a}[@]%%{}}}", self.pattern()),
            10 => format!("${{{a}[@]^^}}"),
            11 => format!("${{{a}[@]@U}}"),
            12 => format!("${{{a}}}"),
            13 => format!("${{#{a}[{}]}}", self.n(0, 3)),
            14 => format!("${{{a}[@]{}{}}}", self.pick(&["+", "-"]), self.pick(&["a", "X", "_", "1"])),
            _ => format!("${{A[{}]}}", self.pick(&["k", "x", "'a b'", "z"])),
        }
    }

    fn positional(&mut self) -> String {
        self.pick(&["$1", "$2", "$#", "$@", "$*", "${@:2}", "${*:1:2}", "${#1}", "${@: -1}", "${1:-d}", "$3", "${@/a/X}", "$?"])
    }

    fn value_lit_or_plain(&mut self) -> String {
        if self.p(0.5) {
            self.pick(&["a", "X", "_", "1", "ab", "", "z"])
        } else {
            self.value_lit()
        }
    }

    /// One part of a word. `inq` = we are inside double quotes.
    fn word_part(&mut self, d: usize, inq: bool) -> String {
        let k = self.n(0, 19);
        match k {
            0..=1 => {
                if inq {
                    self.pick(&["a", " ", "b c", "*", ":", "-", "x y", "\\$", "\\\""])
                } else {
                    self.value_lit()
                }
            }
            2 => {
                if inq {
                    self.pick(&["a", "z", "-"])
                } else {
                    self.pick(&["a", "z", "-", "*", "f*", "?.txt", "[fg]*", "*.@(txt|log)", "sub/*", ".*", "\\*", "~", "{a,b}", "{1..3}", "{a..e..2}", "{01..3}", "{c,{d,e}f}", "{3..1}", "a{,b}"])
                }
            }
            3..=7 => self.param_exp(d),
            8..=10 if !self.no_at => self.array_exp(),
            8..=10 => format!("${{{}[{}]}}", self.pick(ARRAYS), self.n(0, 2)),
            11 if !self.no_at => self.positional(),
            11 => self.pick(&["$1", "$#", "${#1}", "${1:-d}", "$*"]),
            12 | 13 => format!("$(({}))", self.arith(2)),
            14 if d > 0 => {
                let inner = self.word(d - 1);
                format!("$(echo {inner})")
            }
            15 if d > 0 => {
                let inner = self.word(d - 1);
                format!("$(printf '%s\\n' {inner} {})", self.pick(&["", "''", "x"]))
            }
            16 if !inq && d > 0 => {
                let k = self.n(1, 2);
                let was = self.no_at;
                self.no_at = was || k > 1;
                let parts: Vec<String> = (0..k).map(|_| self.word_part(d - 1, true)).collect();
                self.no_at = was;
                format!("\"{}\"", parts.join(""))
            }
            17 if !inq => self.pick(&["$'a\\tb'", "$'\\x41'", "$'a\\nb'", "$'\\101'"]),
            _ => self.param_exp(d),
        }
    }

    pub fn word(&mut self, d: usize) -> String {
        let quoted = self.p(0.45);
        let n = if self.p(0.6) { 1 } else { self.n(2, 3) };
        let was = self.no_at;
        self.no_at = was || n > 1;
        let parts: Vec<String> = (0..n).map(|_| self.word_part(d, quoted)).collect();
        self.no_at = was;
        let w = parts.join("");
        if quoted {
            format!("\"{w}\"")
        } else if w.is_empty() {
            "''".into()
        } else {
            w
        }
    }

    fn words(&mut self, lo: i64, hi: i64) -> String {
        let n = self.n(lo, hi);
        (0..n).map(|_| self.word(2)).collect::<Vec<_>>().join(" ")
    }

    // ---------- statements ----------

    fn show(&mut self) -> String {
        format!("printf '<%s>' {}; echo", self.words(0, 4))
    }

    fn cond(&mut self) -> String {
        match self.n(0, 11) {
            0 => format!("[[ {} == {} ]]", self.word(1), self.pattern()),
            1 => format!("[[ {} != {} ]]", self.word(1), self.word(1)),
            2 => format!("[[ -n {} ]]", self.word(1)),
            3 => format!("[[ -z {} || {} = {} ]]", self.word(1), self.word(1), self.pattern()),
            4 => format!("(( {} ))", self.arith(2)),
            5 => format!("[ {} = {} ]", self.word(1), self.word(1)),
            6 => format!("[[ {} -{} {} ]]", self.pick(&["n", "1", "-2", "0x1f", "${#v}", "n+1"]), self.pick(&["lt", "eq", "ge", "ne"]), self.arith_atom()),
            7 => format!("[[ {} =~ {} ]]", self.word(1), self.pick(&["^a", "b$", "a|b", "^(a)(b)?", "[[:digit:]]+", "' '", "x*", "\\*"])),
            8 => format!("[[ -{} {} ]]", self.pick(&["f", "e", "d", "s"]), sq(self.s.pick(FILES))),
            9 => format!("test {} {} {}", self.word(1), self.pick(&["=", "!=", "<", ">"]), self.word(1)),
            10 if !self.callable().is_empty() => {
                let f = self.s.pick(&self.callable()).clone();
                format!("{f} {}", self.words(0, 2))
            }
            _ => self.pick(&["true", "false", "! true", "[[ ! -e nope ]]", "[ -n \"$v\" -a -z \"$w\" ]"]),
        }
    }

    fn body(&mut self, d: usize) -> Vec<Node> {
        let n = self.n(1, 3);
        (0..n).map(|_| self.stmt(d)).collect()
    }

    pub fn stmt(&mut self, d: usize) -> Node {
        let compound = d > 0 && self.p(0.3);
        if compound {
            return self.compound(d - 1);
        }
        Node::leaf(self.simple())
    }

    fn simple(&mut self) -> String {
        let k = self.n(0, 43);
        let s = match k {
            0..=3 => self.show(),
            4 => format!("echo {}", self.words(0, 3)),
            5..=7 => {
                let v = self.pick(SCALARS);
                format!("{v}{}{}", self.pick(&["=", "=", "+="]), self.word(2))
            }
            8 => {
                let a = self.pick(ARRAYS);
                format!("{a}{}({})", self.pick(&["=", "=", "+="]), self.words(1, 4))
            }
            9 => {
                let a = self.pick(ARRAYS);
                format!("{a}[{}]{}{}", self.pick(&["0", "1", "2", "n", "k", "5"]), self.pick(&["=", "+="]), self.word(1))
            }
            10 => format!("unset {}", self.pick(&["'a[1]'", "'b[1]'"])),
            // rhs pure: `n = (n*=2) + 1` gives the inner assignment's value in bash (no
            // sequence-point rule documented), so no nested assignments to n here
            11 => match self.n(0, 2) {
                0 => format!("n=$(( {} ))", self.arith(2)),
                1 => format!("(( n = ({}) ))", self.arith(2)),
                _ => format!("let \"n = ({})\"", self.arith(2)),
            },
            12 => format!("n+={}", if self.p(0.5) { format!("$(( {} ))", self.arith(1)) } else { self.n(-5, 20).to_string() }),
            13 => { self.pure = false; format!("echo $(( {} ))", self.arith(3)) }
            14 => { self.pure = false; format!("(( {} )); echo \"rc=$?\"", self.arith(2)) }
            15 => {
                let fmt = self.pick(&[
                    "'%s|'", "'%5s|'", "'%-4s|'", "'%d,'", "'%x %o\\n'", "'%03d\\n'", "'%.2s:'", "'%c'", "'%b\\n'",
                    "'%i %%\\n'", "'[%*s]' 3", "'%s=%s\\n'", "'%X:'", "'%+d '", "'%#x\\n'", "'%u\\n'",
                ]);
                // numeric formats get numeric arguments: brush writes its "expected a numeric
                // value" warning past `2>/dev/null` (notes), so non-numeric args stay rare
                let numeric = !fmt.contains('s') && !fmt.contains('b') && !fmt.contains('c');
                let args = if numeric && self.p(0.97) {
                    (0..self.n(0, 3)).map(|_| format!("$(( {} ))", self.arith(1))).collect::<Vec<_>>().join(" ")
                } else {
                    self.words(0, 3)
                };
                format!("printf {fmt} {args}; echo")
            }
            16 => format!("set -- {}", self.words(0, 4)),
            // bash.1 (shift): "The return status is greater than zero if n is greater than $#";
            // only the sign is specified (brush returns 2, bash 1).
            17 => format!("shift {}; echo \"rc=$(( $? > 0 )) $#\"", self.n(0, 2)),
            18 => {
                // whitespace-only IFS: non-whitespace IFS edge cases are documented as known
                // differences (docs/reference/compatibility.md)
                // (and not null IFS: bash joins unquoted ${a[@]#p} fields then, brush does not)
                let v = self.pick(&["IFS=' '", "unset IFS", "IFS=$' \\t\\n'"]);
                v
            }
            19 => format!("read -r {} <<< {}; echo \"rc=$?\"", self.pick(&["v w", "v", "v w x", "a1 x", "-a a"]).replace("a1 x", "x w"), self.word(2)),
            20 => format!("IFS={} read -r {} <<< {}", self.pick(&["' '", "''", "$'\\t'"]), self.pick(&["v w", "v", "-a b"]), self.word(1)),
            // bash (and zsh) expand redirections of an external command in the forked child, so a
            // `${w:=x}` there does not assign in the shell; dash and brush assign. Neither bash.1 nor
            // POSIX promise either, so keep assigning expansions out of this here-string (as in heredocs).
            21 => {
                self.in_heredoc = true;
                let w = self.word(2);
                self.in_heredoc = false;
                format!("cat <<< {w}")
            }
            22 if !self.in_cmdsub => {
                let lines: Vec<String> = (0..self.n(1, 3)).map(|_| self.heredoc_line()).collect();
                let quoted = self.p(0.3);
                let dash = self.p(0.2);
                let lines = if dash { lines.iter().map(|l| format!("\t{l}")).collect() } else { lines };
                format!(
                    "cat <<{}{}\n{}\nEOF",
                    if dash { "-" } else { "" },
                    if quoted { "'EOF'" } else { "EOF" },
                    lines.join("\n")
                )
            }
            23 => format!("{} {} {}", self.pick(&["echo", "printf '%s\\n'"]), self.word(1), self.redir()),
            // never send stderr into stdout or files: message texts differ by design
            24 => format!("cat {} 2>/dev/null; echo \"rc=$?\"", sq(self.s.pick(FILES))),
            25 => format!("{} 2>/dev/null; echo \"rc=$?\"", self.cond()),
            26 => format!("{}; echo \"rc=$?\"", self.cond()),
            // globstar `**` does not recurse in brush (notes): keep it rare
            27 if self.p(0.1) => "shopt -s globstar".into(),
            27 => self
                .pick(&[
                    "shopt -s nullglob", "shopt -u nullglob", "shopt -s dotglob", "shopt -s nocaseglob",
                    "set -f", "set +f",
                ]),
            // declare's arguments get brace expansion in bash, not in brush (notes): no braces
            28 => format!("declare {} {}={}", self.pick(&["-l", "-u"]), self.pick(&["v", "w"]), self.value_lit()),
            29 => format!("{} | {}", self.show_or_echo(), self.pick(&["cat", "wc -l", "tr a-z A-Z", "sort", "head -n 1", "while read -r l; do echo \"[$l]\"; done"])),
            30 if !self.callable().is_empty() => {
                let f = self.s.pick(&self.callable()).clone();
                format!("{f} {}; echo \"rc=$?\"", self.words(0, 3))
            }
            31 => format!("v=$({})", self.show_or_echo()),
            32 => format!("mapfile -t {} <<< {}", self.pick(ARRAYS), self.word(1)),
            33 => format!("printf '<%s>' {}; echo", self.pick(&["\"${a[@]}\"", "\"${b[@]}\"", "\"$@\"", "$*", "\"${!A[@]}\""]).replace("\"${!A[@]}\"", "\"${A[k]}\"")),
            34 => format!("A[{}]={}", self.pick(&["k", "x", "'a b'", "z"]), self.word(1)),
            // `break n` with n > depth ends the whole script in brush (bash.1: "all enclosing
            // loops are exited"): see notes. Keep n <= depth.
            35 if self.loop_depth > 0 => {
                let n = self.n(1, self.loop_depth as i64);
                format!("{} {n}", self.pick(&["break", "continue"]))
            }
            // like declare (28): bash brace-expands local's assignment arguments, brush does not
            // (upstream #1569, known): keep brace expansions out of local's value
            36 if self.in_func => {
                let mut w = self.word(1);
                for _ in 0..8 {
                    if !w.char_indices().any(|(i, c)| c == '{' && (i == 0 || w.as_bytes()[i - 1] != b'$')) {
                        break;
                    }
                    w = self.word(1);
                }
                if w.char_indices().any(|(i, c)| c == '{' && (i == 0 || w.as_bytes()[i - 1] != b'$')) {
                    w = self.value_lit();
                }
                format!("local {}={}", self.pick(SCALARS), w)
            }
            37 if self.in_func => format!("return {}", self.n(0, 3)),
            38 => format!("echo {} > {}; cat {}", self.word(1), sq(self.s.pick(FILES)), sq(self.s.pick(FILES))),
            39 => format!("for f in {}; do echo \"[$f]\"; done", self.pick(&["*", "*.txt", "f*", ".*", "*/*", "**", "[a-f]*", "?(f)*", "!(*.txt)", "*.{txt,log}"])),
            41 | 42 if !self.funcs.is_empty() && !self.in_func => self.roundtrip_func(),
            41..=43 => self.roundtrip_quote(),
            _ => self.show(),
        };
        self.pure = true;
        s
    }

    /// Round-trip of the function printer: bash.1 (declare -f) "-f ... display ... function
    /// definitions", and the printed definition is shell input (bash re-reads it unchanged). A
    /// definition re-read from `declare -f` must behave the same and print the same again.
    /// Functions a command here may call (see `cur_func`).
    fn callable(&self) -> Vec<String> {
        match &self.cur_func {
            Some(c) => self.funcs.iter().filter(|f| f.as_str() < c.as_str()).cloned().collect(),
            None => self.funcs.clone(),
        }
    }

    fn roundtrip_func(&mut self) -> String {
        let f = self.s.pick(&self.funcs).clone();
        if self.fragile.contains(&f) && !self.p(0.1) {
            return self.roundtrip_quote();
        }
        match self.n(0, 3) {
            0 => format!("( eval \"$(declare -f {f})\"; {f} {}; echo \"rc=$?\" )", self.words(0, 2)),
            1 => format!("eval \"$(declare -f {f})\"; echo \"rc=$?\""),
            2 => format!("[[ \"$(declare -f {f})\" == \"$(eval \"$(declare -f {f})\"; declare -f {f})\" ]]; echo \"fp=$?\""),
            _ => "declare -F".into(),
        }
    }

    /// Round-trip of quoting: bash.1 says `${p@Q}` is "quoted in a format that can be reused as
    /// input", `printf %q` quotes "in a format that can be reused as shell input", `declare -p`
    /// displays "in a form that may be reused as input", `${p@A}` is "an assignment statement or
    /// declare command that, if evaluated, will recreate parameter". Values may first be set to a
    /// quoting-hostile string.
    fn roundtrip_quote(&mut self) -> String {
        const STRESS: &[&str] = &[
            "'#a'", "'~'", "'~/x'", "'a\\b'", "'!x'", "'`x`'", "'a;b'", "'{a,b}'", "'$(x)'",
            "$'\\x01'", "$'\\x7f'", "'a=b'", "' '", "$'\\r'", "$'a\\nb'", "\"'\"", "'\\'", "'-n'", "'#'",
            "'%q'", "'a  b'", "'[a]*'", "$'\\t'", "'\"'", "'a&b|c'", "'<x>'",
        ];
        let pre = match self.n(0, 3) {
            0 => format!("v={}; ", self.pick(STRESS)),
            // element 0 only: a gap would make the array sparse (a known, separate difference)
            1 => format!("a[0]={}; ", self.pick(STRESS)),
            _ => String::new(),
        };
        let x = self.pick(SCALARS);
        let a = self.pick(ARRAYS);
        let body = match self.n(0, 8) {
            0 => format!("eval \"y=${{{x}@Q}}\"; printf '<%s>' \"$y\"; echo"),
            1 => format!("printf -v y %q \"${x}\"; eval \"y=$y\"; printf '<%s>' \"$y\"; echo"),
            2 => format!("eval \"$(declare -p {x} {a} n)\"; echo \"rc=$?\""),
            3 => format!("eval \"{}=(${{{a}[@]@Q}})\"; printf '<%s>' \"${{{}[@]}}\"; echo", if a == "a" { "b" } else { "a" }, if a == "a" { "b" } else { "a" }),
            4 => "eval \"$(declare -p A)\"; printf '<%s>' \"${A[k]}\" \"${A[x]}\" \"${A[z]}\" \"${#A[@]}\"; echo".into(),
            5 => "eval \"set -- ${@@Q}\"; printf '<%s>' \"$#\" \"$@\"; echo".into(),
            6 => format!("eval \"${{{x}@A}}\"; eval \"${{{a}[@]@A}}\"; echo \"rc=$?\""),
            7 => format!("printf -v y '%q ' \"${{{a}[@]}}\"; eval \"{a}=($y)\"; printf '<%s>' \"${{{a}[@]}}\"; echo"),
            _ => format!("y=$(declare -p {x}); unset {x}; eval \"$y\"; printf '<%s>' \"${x}\"; echo"),
        };
        format!("{pre}{body}")
    }

    fn show_or_echo(&mut self) -> String {
        if self.p(0.5) {
            format!("printf '%s\\n' {}", self.words(1, 3))
        } else {
            format!("echo {}", self.words(1, 2))
        }
    }

    fn redir(&mut self) -> String {
        let f = sq(self.s.pick(FILES));
        match self.n(0, 5) {
            0 => format!("> {f}"),
            1 => format!(">> {f}"),
            2 => format!("2>/dev/null > {f}"),
            3 => format!(">{f} 2>/dev/null"),
            4 => "1>&2 2>/dev/null".to_string(),
            _ => format!("3>&1 >> {f} 2>/dev/null 1>&3 3>&-"),
        }
    }

    fn heredoc_line(&mut self) -> String {
        self.in_heredoc = true;
        let parts: Vec<String> = (0..self.n(1, 3)).map(|_| match self.n(0, 4) {
            0 => self.pick(&["text", "a b", " lead", "'q'", "\"dq\"", "\\$v", "x\\\\y", "*"]),
            1 | 2 => self.param_exp(1),
            3 => format!("$(({}))", self.arith(1)),
            _ => self.array_exp(),
        }).collect();
        self.in_heredoc = false;
        parts.join(" ")
    }

    fn compound(&mut self, d: usize) -> Node {
        let t = |s: String| Seg::Text(s);
        match self.n(0, 9) {
            0 => {
                let c = self.cond();
                let b1 = self.body(d);
                if self.p(0.5) {
                    let b2 = self.body(d);
                    Node(vec![t(format!("if {c}; then")), Seg::Body(b1), t("else".into()), Seg::Body(b2), t("fi".into())])
                } else {
                    Node(vec![t(format!("if {c}; then")), Seg::Body(b1), t("fi".into())])
                }
            }
            1 => {
                let ws = self.words(1, 3);
                let var = self.pick(&["x", "v"]);
                self.loop_depth += 1;
                let b = self.body(d);
                self.loop_depth -= 1;
                Node(vec![t(format!("for {var} in {ws}; do")), Seg::Body(b), t("done".into())])
            }
            2 => {
                let k = self.n(1, 3);
                self.loop_depth += 1;
                let b = self.body(d);
                self.loop_depth -= 1;
                Node(vec![t(format!("for (( k=0; k<{k}; k++ )); do")), Seg::Body(b), t("done".into())])
            }
            3 if !self.in_cmdsub => {
                // bash 5.3 bug (the reference is wrong, not brush): in a case word, a
                // double-quoted segment next to unquoted text keeps a stray byte when an
                // operator expansion of an empty variable ends it: `w=; case -"${w^^}" in -)`
                // and `case "${w:0}"x in x)` don't match (but `case "-${w^^}"` and
                // `x=-"${w^^}"; case $x` do). bash.1: the case word "undergoes tilde expansion,
                // parameter and variable expansion, ... and quote removal". So no case word
                // that mixes a quoted segment holding an expansion with unquoted text.
                let mixed = |w: &str| {
                    let whole = w.len() > 1 && w.starts_with('"') && w.ends_with('"') && w.matches('"').count() == 2;
                    !whole && w.contains('"') && w.contains("${")
                };
                let mut w = self.word(1);
                for _ in 0..8 {
                    if !mixed(&w) {
                        break;
                    }
                    w = self.word(1);
                }
                if mixed(&w) {
                    w = self.value_lit();
                }
                let mut segs = vec![t(format!("case {w} in"))];
                for _ in 0..self.n(1, 3) {
                    let pats: Vec<String> = (0..self.n(1, 2)).map(|_| {
                        let p = self.pattern();
                        if p.is_empty() { "''".into() } else { p }
                    }).collect();
                    segs.push(t(format!("{})", pats.join("|"))));
                    segs.push(Seg::Body(self.body(d)));
                    segs.push(t(self.pick(&[";;", ";;", ";&", ";;&"])));
                }
                segs.push(t("esac".into()));
                Node(segs)
            }
            4 | 5 => {
                let name = self.pick(&["f", "g"]);
                let was = self.in_func;
                self.in_func = true;
                let was_cur = self.cur_func.replace(name.clone());
                let b = self.body(d);
                self.cur_func = was_cur;
                self.in_func = was;
                if !self.funcs.contains(&name) {
                    self.funcs.push(name.clone());
                }
                // definition forms (all printed back by `declare -f`, see roundtrip_func)
                let (open, close) = match self.n(0, 9) {
                    0 => (format!("function {name} {{"), "}".to_string()),
                    1 => (format!("function {name}() {{"), "}".to_string()),
                    2 => (format!("{name}() ("), ")".to_string()),
                    3 if self.p(0.3) => (format!("{name}() {{"), "} 2>/dev/null".to_string()),
                    _ => (format!("{name}() {{"), "}".to_string()),
                };
                let mut text = String::new();
                render_into(&b, &mut text);
                self.fragile.retain(|f| *f != name);
                if close != "}" && close != ")" || text.contains("<<") || text.contains("'a\nb'") || text.contains("$(\n") || compound_redir(&text) {
                    self.fragile.push(name.clone());
                }
                Node(vec![t(open), Seg::Body(b), t(close)])
            }
            6 => {
                let ld = std::mem::replace(&mut self.loop_depth, 0);
                let b = self.body(d);
                self.loop_depth = ld;
                Node(vec![t("(".into()), Seg::Body(b), t(format!("); echo \"rc=$?\""))])
            }
            7 => {
                self.redir_depth += 1;
                let b = self.body(d);
                self.redir_depth -= 1;
                let r = self.redir();
                Node(vec![t("{".into()), Seg::Body(b), t(format!("}} {r}; cat {}", sq(self.s.pick(FILES))))])
            }
            8 => {
                let ld = std::mem::replace(&mut self.loop_depth, 0);
                let b = self.body(d);
                self.loop_depth = ld;
                // A streaming tail writes to the enclosing stdout while the left stage still runs; when
                // that stdout is a file the left stage also writes (`{ { echo 1 > g; } | cat; } > g`),
                // the file content depends on scheduling (bash 5.3 itself varies run to run). Inside a
                // redirected block or a function (callable from one), only use tails that write after EOF.
                let tail = if self.redir_depth > 0 || self.in_func {
                    self.pick(&["sort", "wc -l"])
                } else {
                    self.pick(&["cat", "sort", "while read -r l; do echo \"<$l>\"; done", "wc -l"])
                };
                Node(vec![t("{".into()), Seg::Body(b), t(format!("}} | {tail}"))])
            }
            _ => {
                // no `case` inside $( ): brush fails to parse it (notes)
                let was = self.in_cmdsub;
                self.in_cmdsub = true;
                let ld = std::mem::replace(&mut self.loop_depth, 0);
                let b = self.body(d);
                self.loop_depth = ld;
                self.in_cmdsub = was;
                Node(vec![t("v=$(".into()), Seg::Body(b), t(")".into())])
            }
        }
    }

    /// The prelude: fixed files plus random variable initialisation, as deletable nodes.
    pub fn script(&mut self) -> Vec<Node> {
        // the fixed prelude (files, extglob) is prepended by `render_script`, never deleted
        let mut init: Vec<String> = vec![];
        for v in SCALARS {
            init.push(format!("{v}={}", self.value_lit()));
        }
        for a in ARRAYS {
            if self.p(1.0) {
                let n = self.n(1, 4);
                let vals: Vec<String> = (0..n).map(|_| self.value_lit()).collect();
                init.push(format!("{a}=({})", vals.join(" ")));
            }
        }
        init.push(format!("declare -i n={}", self.n(-2, 5)));
        init.push("declare -A A=([k]=K [x]='x y')".into());
        if self.p(0.5) {
            init.push(format!("set -- {}", self.words(0, 3)));
        }
        // node 0 holds the variable initialisation; the shrinker never deletes it
        let mut out = vec![Node::leaf(init.join("\n"))];
        let n = self.n(3, 12);
        for _ in 0..n {
            out.push(self.stmt(2));
        }
        out.push(Node::leaf("printf '<%s>' \"$v\" \"$w\" \"$x\" \"$n\" \"${a[@]}\" \"${b[@]}\" \"$#\" \"$*\"; echo"));
        out
    }
}
