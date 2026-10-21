// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! URLPattern cases: [`PatternCase`] (a constructor string or init, an
//! optional base, `ignoreCase`, and inputs to test and exec), run on ada
//! ([`ada`]) and on urlpattern-polyfill ([`reference`]), and drawn by
//! [`pattern_cases`].

use std::ops::ControlFlow;

use ephyra::{Gen, Source};
use serde_json::{json, Value};

use crate::{gen, params::text};

/// The component names, in the order of `URLPatternInit` and of
/// [`PatternResult::components`]; an init also has `baseURL`.
pub const COMPONENTS: [&str; 8] =
    ["protocol", "username", "password", "hostname", "port", "pathname", "search", "hash"];

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Spec {
    /// `new URLPattern(string, base?)`.
    Text(String),
    /// `new URLPattern({protocol, ..., hash, baseURL})`, absent fields `None`.
    Init([Option<String>; 9]),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PatternCase {
    pub spec: Spec,
    pub base: Option<String>,
    pub ignore_case: bool,
    /// URL strings to test and exec, each with an optional base.
    pub inputs: Vec<(String, Option<String>)>,
}

/// One input: `test` and `exec` (0 false or no match, 1 true or a match,
/// 2 an error, 3 for ada: std::regex gave up, so no answer), and for a match, per component its input and its groups as
/// alternating names and values (sorted by name; `\0` for undefined).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputResult {
    pub test: u8,
    pub exec: u8,
    pub groups: Vec<(String, Vec<String>)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatternResult {
    pub ok: bool,
    pub components: Vec<String>,
    pub regexp_groups: bool,
    pub inputs: Vec<InputResult>,
}

extern "C" {
    fn ea_pattern(bytes: *const u8, len: usize, out: *mut *mut u8) -> usize;
    fn ea_free(p: *mut u8);
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn byte(&mut self) -> u8 {
        let b = self.0[0];
        self.0 = &self.0[1..];
        b
    }

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

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn put_opt(out: &mut Vec<u8>, s: &Option<String>) {
    out.push(u8::from(s.is_some()));
    put_str(out, s.as_deref().unwrap_or(""));
}

/// The result of `case` in ada.
pub fn ada(case: &PatternCase) -> PatternResult {
    let mut bytes = Vec::new();
    match &case.spec {
        Spec::Text(t) => {
            bytes.push(0);
            put_str(&mut bytes, t);
        },
        Spec::Init(fields) => {
            bytes.push(1);
            for f in fields {
                put_opt(&mut bytes, f);
            }
        },
    }
    put_opt(&mut bytes, &case.base);
    bytes.push(u8::from(case.ignore_case));
    bytes.extend_from_slice(&(case.inputs.len() as u32).to_le_bytes());
    for (input, base) in &case.inputs {
        put_str(&mut bytes, input);
        put_opt(&mut bytes, base);
    }
    let mut out = std::ptr::null_mut();
    // SAFETY: `bytes` is live for the call; the shim writes a malloc'd
    // buffer of the returned length to `out`, copied and then freed.
    let t = crate::ada::locked(|| unsafe {
        let len = ea_pattern(bytes.as_ptr(), bytes.len(), &mut out);
        let t = std::slice::from_raw_parts(out, len).to_vec();
        ea_free(out);
        t
    });
    let mut r = Reader(&t);
    if r.byte() == 0 {
        return PatternResult {
            ok: false,
            components: Vec::new(),
            regexp_groups: false,
            inputs: Vec::new(),
        };
    }
    let components = COMPONENTS.iter().map(|_| r.str()).collect();
    let regexp_groups = r.byte() == 1;
    let inputs = case
        .inputs
        .iter()
        .map(|_| {
            let test = r.byte();
            let exec = r.byte();
            let groups = if exec == 1 {
                COMPONENTS.iter().map(|_| (r.str(), r.list())).collect()
            } else {
                Vec::new()
            };
            InputResult { test, exec, groups }
        })
        .collect();
    PatternResult { ok: true, components, regexp_groups, inputs }
}

/// The result of `case` in urlpattern-polyfill. It has no
/// `hasRegExpGroups`; that field is copied from nothing and left false.
pub fn reference(case: &PatternCase) -> PatternResult {
    reference_inner(case, None).expect("an answer from the polyfill")
}

/// [`reference`], or `None` when the polyfill takes longer than `limit`
/// (its RegExp backtracking; see [`crate::reference::request_within`]).
pub fn reference_within(case: &PatternCase, limit: std::time::Duration) -> Option<PatternResult> {
    reference_inner(case, Some(limit))
}

fn reference_inner(case: &PatternCase, limit: Option<std::time::Duration>) -> Option<PatternResult> {
    let spec = match &case.spec {
        Spec::Text(t) => json!(t),
        Spec::Init(fields) => {
            let mut o = serde_json::Map::new();
            for (name, f) in COMPONENTS.iter().chain(&["baseURL"]).zip(fields) {
                if let Some(v) = f {
                    o.insert((*name).to_owned(), json!(v));
                }
            }
            Value::Object(o)
        },
    };
    let inputs: Vec<Value> = case.inputs.iter().map(|(i, b)| json!([i, b])).collect();
    let request = json!({ "pattern": spec, "base": case.base, "ignoreCase": case.ignore_case, "inputs": inputs });
    let v = match limit {
        Some(d) => crate::reference::request_within(&request, d)?,
        None => crate::reference::request(&request),
    };
    if !v["ok"].as_bool().unwrap() {
        return Some(PatternResult {
            ok: false,
            components: Vec::new(),
            regexp_groups: false,
            inputs: Vec::new(),
        });
    }
    let strings = |v: &Value| {
        v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_owned()).collect::<Vec<_>>()
    };
    Some(PatternResult {
        ok: true,
        components: strings(&v["components"]),
        regexp_groups: false,
        inputs: v["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| InputResult {
                test: i["test"].as_u64().unwrap() as u8,
                exec: i["exec"].as_u64().unwrap() as u8,
                groups: i["groups"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|c| (c[0].as_str().unwrap().to_owned(), strings(&c[1])))
                    .collect(),
            })
            .collect(),
    })
}

/// The result of `case` in the `urlpattern` crate (Deno's implementation,
/// on the Rust `regex` crate), or `None` when that crate panics on the
/// pattern. It panics on some crafted patterns (a subtract-with-overflow in
/// its constructor-string parser, e.g. `*:]`); that is a bug in the
/// reference tool, not ada, so a caller treats `None` as "no reference
/// answer" and skips the comparison.
pub fn deno(case: &PatternCase) -> Option<PatternResult> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| deno_inner(case))).ok()
}

fn deno_inner(case: &PatternCase) -> PatternResult {
    use urlpattern::{UrlPattern, UrlPatternInit, UrlPatternMatchInput, UrlPatternOptions};
    let failed = PatternResult {
        ok: false,
        components: Vec::new(),
        regexp_groups: false,
        inputs: Vec::new(),
    };
    let parse_url = |s: &str| url::Url::parse(s).ok();
    let init = match &case.spec {
        Spec::Text(t) => {
            let base = match &case.base {
                Some(b) => match parse_url(b) {
                    Some(u) => Some(u),
                    None => return failed,
                },
                None => None,
            };
            match UrlPatternInit::parse_constructor_string::<CaseRegex>(t, base) {
                Ok(i) => i,
                Err(_) => return failed,
            }
        },
        Spec::Init(f) => {
            let base_url = match &f[8] {
                Some(b) => match parse_url(b) {
                    Some(u) => Some(u),
                    None => return failed,
                },
                None => None,
            };
            UrlPatternInit {
                protocol: f[0].clone(),
                username: f[1].clone(),
                password: f[2].clone(),
                hostname: f[3].clone(),
                port: f[4].clone(),
                pathname: f[5].clone(),
                search: f[6].clone(),
                hash: f[7].clone(),
                base_url,
            }
        },
    };
    let options = UrlPatternOptions { ignore_case: case.ignore_case, ..Default::default() };
    let Ok(p) = UrlPattern::<CaseRegex>::parse(init, options) else { return failed };
    let components = vec![
        p.protocol().to_owned(),
        p.username().to_owned(),
        p.password().to_owned(),
        p.hostname().to_owned(),
        p.port().to_owned(),
        p.pathname().to_owned(),
        p.search().to_owned(),
        p.hash().to_owned(),
    ];
    let inputs = case
        .inputs
        .iter()
        .map(|(input, base)| {
            let url = match base {
                Some(b) => url::Url::parse(b).ok().and_then(|b| b.join(input).ok()),
                None => parse_url(input),
            };
            let Some(url) = url else {
                return InputResult { test: 0, exec: 0, groups: Vec::new() };
            };
            let test = match p.test(UrlPatternMatchInput::Url(url.clone())) {
                Ok(t) => u8::from(t),
                Err(_) => 2,
            };
            let (exec, groups) = match p.exec(UrlPatternMatchInput::Url(url)) {
                Ok(Some(m)) => {
                    let cs = [
                        m.protocol, m.username, m.password, m.hostname, m.port, m.pathname,
                        m.search, m.hash,
                    ];
                    let groups = cs
                        .into_iter()
                        .map(|c| {
                            let mut g: Vec<(String, Option<String>)> =
                                c.groups.into_iter().collect();
                            g.sort();
                            (
                                c.input,
                                g.into_iter()
                                    .flat_map(|(k, v)| [k, v.unwrap_or_else(|| "\0".to_owned())])
                                    .collect(),
                            )
                        })
                        .collect();
                    (1, groups)
                },
                Ok(None) => (0, Vec::new()),
                Err(_) => (2, Vec::new()),
            };
            InputResult { test, exec, groups }
        })
        .collect();
    PatternResult { ok: true, components, regexp_groups: p.has_regexp_groups(), inputs }
}

/// The `urlpattern` crate's `regex::Regex` engine, except that it honours
/// the `i` flag: the crate's own impl drops the flags, so under
/// `ignoreCase` every component that falls back to a regexp matched case
/// sensitively. (Its literal prefix and suffix checks in `Matcher::matches`
/// are case sensitive too; the majority property masks that.)
struct CaseRegex(regex::Regex);

impl urlpattern::regexp::RegExp for CaseRegex {
    fn syntax() -> urlpattern::RegexSyntax {
        urlpattern::RegexSyntax::Rust
    }

    fn parse(pattern: &str, flags: &str, _force_eval: bool) -> Result<Self, ()> {
        regex::RegexBuilder::new(pattern).case_insensitive(flags.contains('i')).build().map(CaseRegex).map_err(|_| ())
    }

    fn matches<'a>(&self, text: &'a str) -> Option<Vec<Option<&'a str>>> {
        let c = self.0.captures(text)?;
        Some(c.iter().skip(1).map(|m| m.map(|m| m.as_str())).collect())
    }

    fn pattern_string(&self) -> &str {
        self.0.as_str()
    }
}

const NAMES: &[&str] = &["id", "x", "name", "_a", "$b", "é", "café", "ab1", "Ω"];

/// A component pattern from the URLPattern token grammar: literal text
/// (with characters the component canonicalizes or encodes), named groups
/// with and without custom regexps, regexp groups, wildcards, `{...}`
/// groups, modifiers, and escapes.
fn component(g: &mut Source, literals: &[&str]) -> String {
    let mut out = String::new();
    let mut names: Vec<&str> = Vec::new();
    for _ in 0..1 + gen::few(g, 4) {
        let modifier = |g: &mut Source| {
            if g.bool_with(0.25) {
                gen::pick_str(g, &["?", "+", "*"])
            } else {
                String::new()
            }
        };
        let part = match g.int(0u32..21) {
            0..=7 => gen::pick_str(g, literals),
            8..=11 => {
                let free: Vec<&&str> = NAMES.iter().filter(|n| !names.contains(n)).collect();
                let n = if free.is_empty() { "zz" } else { **g.pick(&free) };
                names.push(n);
                let re = if g.bool_with(0.3) {
                    format!("({})", regexp(g))
                } else {
                    String::new()
                };
                format!(":{n}{re}{}", modifier(g))
            },
            12..=13 => format!("({}){}", regexp(g), modifier(g)),
            14..=15 => format!("*{}", modifier(g)),
            16..=17 => format!("{{{}}}{}", gen::pick_str(g, literals), modifier(g)),
            18 => gen::pick_str(g, &["\\:", "\\*", "\\(", "\\{", "\\\\", "\\.", "\\/"]),
            19 => gen::pick_str(
                g,
                // Shapes that drive the tokenizer's error and edge branches:
                // a group name that is empty or starts with a digit or a
                // non-name char, nested and unbalanced groups, an open group
                // or regexp with no close, a regexp with escapes and nested
                // parens, `:` with a `{`-delimited name, and a named group
                // whose suffix starts with an (escaped) name character.
                &[
                    ":",
                    ":1a",
                    ":-",
                    ":{a}",
                    "::",
                    "(",
                    ")",
                    "((a))",
                    "(a(b))",
                    "{",
                    "}",
                    "{{a}}",
                    "{a",
                    "(a",
                    "(\\)",
                    "([(])",
                    "(a\\(b)",
                    ":\u{e9}",
                    ":\u{200d}",
                    "(?:a)",
                    "(?<n>a)",
                    "*?+",
                    "(a\\",
                    "(a(",
                    "(\\\u{e9})",
                    "(\u{e9})",
                    "(a(?:b)",
                    "((?",
                    "{:id\\x}",
                    "{a:id\\.b}?",
                    "{:id\\_}",
                ],
            ),
            // `'`, `"`, `<`, `>` and `` ` `` are where the URL Standard's
            // percent-encode sets differ (query vs special-query, path,
            // fragment), so each component's canonicalization meets them.
            _ => gen::pick_str(
                g,
                &[
                    "%20", "%41", " ", "é", "%", "#", "?", ":", "@", "[", "]", "^", "|", "\u{1F600}", "'", "\"",
                    "<", ">", "`",
                ],
            ),
        };
        out.push_str(&part);
    }
    out
}

/// The body of a regexp group: mostly one of the common route regexps (and
/// the pathname's and hostname's segment-wildcard regexps, which make the
/// group a nameless segment wildcard that the pattern string writes back),
/// otherwise one drawn from a small ECMAScript grammar (literals, `.`,
/// classes, `\d`/`\w`/`\s` and escaped delimiters, alternation, quantifiers
/// including counted and lazy ones, and nested `(?:`, `(?=` and `(?!`
/// groups). The tokenizer scans a regexp by its own rules (escapes, depth,
/// and a nested `(` that must be followed by `?`), and the compiled pattern
/// runs on the regex engine, so both see shapes beyond a fixed list. Kept to
/// syntax that ECMAScript with the `v` flag and `std::regex` both accept.
fn regexp(g: &mut Source) -> String {
    if g.bool_with(0.5) {
        return gen::pick_str(
            g,
            &["\\d+", "[a-z]+", "a|b", "[^/]+", ".*", "[a-z]{2}", "x?", "[^\\/]+?", "[^\\.]+?"],
        );
    }
    regexp_alt(g, 0)
}

fn regexp_alt(g: &mut Source, depth: u32) -> String {
    let n = 1 + usize::from(g.bool_with(0.25));
    (0..n).map(|_| regexp_seq(g, depth)).collect::<Vec<_>>().join("|")
}

fn regexp_seq(g: &mut Source, depth: u32) -> String {
    let mut out = String::new();
    for _ in 0..1 + gen::few(g, 3) {
        let atom = match g.int(0u32..10) {
            0..=2 => gen::pick_str(g, &["a", "b", "x", "1", "-", "_", "~", "=", "%"]),
            3 => ".".to_owned(),
            4..=5 => gen::pick_str(
                g,
                &["[a-z]", "[^/]", "[0-9a-f]", "[\\d.]", "[^a]", "[A-Z_]", "[\\/]", "[\\w_]"],
            ),
            6 => gen::pick_str(g, &["\\d", "\\w", "\\s", "\\D", "\\/", "\\.", "\\?"]),
            _ if depth < 2 => {
                let open = *g.pick(&["(?:", "(?:", "(?=", "(?!"]);
                format!("{open}{})", regexp_alt(g, depth + 1))
            },
            _ => "a".to_owned(),
        };
        out.push_str(&atom);
        // A quantified lookahead is an error under the `u` and `v` flags.
        if !atom.starts_with("(?=") && !atom.starts_with("(?!") && g.bool_with(0.3) {
            out.push_str(&gen::pick_str(g, &["?", "*", "+", "{2}", "{1,3}", "{0,}", "+?", "*?"]));
        }
    }
    out
}

const HOST_LITERALS: &[&str] =
    &["example.com", "a", ".", "-", "xn--nxasmq6b", "EXAMPLE", "[::1]", "127.0.0.1", "ex", "com"];
const PATH_LITERALS: &[&str] =
    &["/", "/a", "/users", "/b/", "a", "/x.y", "/.", "/..", "/A", "/caf%C3%A9"];
const TEXT_LITERALS: &[&str] = &["a", "b", "q=1", "A", "x", "1", "-", "&", "="];

fn component_for(g: &mut Source, k: usize) -> String {
    match k {
        0 => {
            if g.bool_with(0.6) {
                gen::pick_str(
                    g,
                    &[
                        "https",
                        "http",
                        "*",
                        "http{s}?",
                        "ws",
                        "file",
                        "foo",
                        "(https|http)",
                        "data",
                        "HTTPS",
                    ],
                )
            } else {
                component(g, &["http", "s", "ftp", "a"])
            }
        },
        3 => component(g, HOST_LITERALS),
        4 => {
            if g.bool_with(0.6) {
                gen::pick_str(g, &["", "8080", "*", "80", "443", ":p(\\d+)", "0", "65536"])
            } else {
                component(g, &["8", "0", "1"])
            }
        },
        5 => component(g, PATH_LITERALS),
        _ => component(g, TEXT_LITERALS),
    }
}

fn spec(g: &mut Source) -> (Spec, Option<String>) {
    if g.bool() {
        let mut fields: [Option<String>; 9] = Default::default();
        for (k, f) in fields.iter_mut().enumerate().take(8) {
            if g.bool_with(0.35) {
                *f = Some(component_for(g, k));
            }
        }
        if g.bool_with(0.15) {
            fields[8] = Some(gen::pick_str(
                g,
                &["https://example.com/a/b", "http://h:8/x?y#z", "foo:bar", "file:///C:/"],
            ));
        }
        return (Spec::Init(fields), None);
    }
    // A constructor string: absolute, or relative with a base.
    if g.bool_with(0.7) {
        let mut s = component_for(g, 0);
        s.push_str("://");
        if g.bool_with(0.2) {
            s.push_str(&component_for(g, 1));
            s.push('@');
        }
        s.push_str(&component_for(g, 3));
        if g.bool_with(0.3) {
            s.push(':');
            s.push_str(&component_for(g, 4));
        }
        s.push_str(&component_for(g, 5));
        if g.bool_with(0.3) {
            s.push('?');
            s.push_str(&component_for(g, 6));
        }
        if g.bool_with(0.2) {
            s.push('#');
            s.push_str(&component_for(g, 7));
        }
        (Spec::Text(s), None)
    } else {
        let s = component_for(g, 5);
        let base = gen::pick_str(
            g,
            &["https://example.com/a/b", "http://h:8/x?y#z", "https://example.com", "data:x"],
        );
        (Spec::Text(s), Some(base))
    }
}

/// An input standing for the pattern: its literal text with groups and
/// wildcards filled in (roughly: tokens are replaced, not parsed), or a URL
/// from the URL grammar.
fn input(g: &mut Source, spec: &Spec) -> (String, Option<String>) {
    let fill = |g: &mut Source, s: &str| -> String {
        let mut out = String::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                ':' if chars
                    .peek()
                    .is_some_and(|c| c.is_alphanumeric() || *c == '_' || *c == '$') =>
                {
                    while chars
                        .peek()
                        .is_some_and(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
                    {
                        chars.next();
                    }
                    out.push_str(&gen::pick_str(g, &["1", "42", "abc", "a", "b", "é", ""]));
                },
                '(' => {
                    let mut depth = 1;
                    for c in chars.by_ref() {
                        depth += i32::from(c == '(') - i32::from(c == ')');
                        if depth == 0 {
                            break;
                        }
                    }
                    out.push_str(&gen::pick_str(g, &["1", "a", "ab", "b", ""]));
                },
                '*' => out.push_str(&gen::pick_str(g, &["", "x", "a/b", "c.d"])),
                '{' | '}' | '?' | '+' if g.bool() => {},
                '\\' => {
                    if let Some(n) = chars.next() {
                        out.push(n);
                    }
                },
                _ => out.push(c),
            }
        }
        out
    };
    if g.bool_with(0.25) {
        return (gen::absolute(g), None);
    }
    match spec {
        Spec::Text(t) if t.contains("://") => (fill(g, t), None),
        Spec::Text(t) => (
            fill(g, t),
            Some(gen::pick_str(g, &["https://example.com/a/b", "https://example.com"])),
        ),
        Spec::Init(f) => {
            let proto = f[0]
                .as_ref()
                .map(|p| fill(g, p))
                .filter(|p| !p.is_empty() && !p.contains(['/', ':']))
                .unwrap_or_else(|| "https".into());
            let host = f[3].as_ref().map(|h| fill(g, h)).unwrap_or_else(|| "example.com".into());
            let port = f[4]
                .as_ref()
                .map(|p| fill(g, p))
                .filter(|p| !p.is_empty())
                .map(|p| format!(":{p}"))
                .unwrap_or_default();
            let path = f[5].as_ref().map(|p| fill(g, p)).unwrap_or_default();
            let search = f[6].as_ref().map(|s| format!("?{}", fill(g, s))).unwrap_or_default();
            let hash = f[7].as_ref().map(|h| format!("#{}", fill(g, h))).unwrap_or_default();
            let path =
                if path.is_empty() || path.starts_with('/') { path } else { format!("/{path}") };
            (format!("{proto}://{host}{port}{path}{search}{hash}"), None)
        },
    }
}

/// URLPattern cases: see the module documentation. Shrinking drops inputs
/// and init fields and deletes characters; mutation redraws a part.
pub struct PatternCases;

fn strings_mut(case: &mut PatternCase) -> Vec<&mut String> {
    let mut out: Vec<&mut String> = Vec::new();
    match &mut case.spec {
        Spec::Text(t) => out.push(t),
        Spec::Init(f) => out.extend(f.iter_mut().flatten()),
    }
    out.extend(case.inputs.iter_mut().map(|(i, _)| i));
    out
}

impl Gen for PatternCases {
    type Value = PatternCase;

    fn generate(&self, g: &mut Source) -> PatternCase {
        let (spec, base) = spec(g);
        let n = 1 + gen::few(g, 4);
        let inputs = (0..n).map(|_| input(g, &spec)).collect();
        PatternCase { spec, base, ignore_case: g.bool_with(0.15), inputs }
    }

    fn shrink(
        &self,
        case: &PatternCase,
        visit: &mut dyn FnMut(PatternCase) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        for k in (0..case.inputs.len()).rev() {
            let mut out = case.clone();
            out.inputs.remove(k);
            visit(out)?;
        }
        if let Spec::Init(f) = &case.spec {
            for k in 0..9 {
                if f[k].is_some() {
                    let mut out = case.clone();
                    if let Spec::Init(g) = &mut out.spec {
                        g[k] = None;
                    }
                    visit(out)?;
                }
            }
        }
        if case.ignore_case {
            visit(PatternCase { ignore_case: false, ..case.clone() })?;
        }
        let mut probe = case.clone();
        let n = strings_mut(&mut probe).len();
        for at in 0..n {
            let s = strings_mut(&mut case.clone())[at].clone();
            gen::deletions(&s, &mut |t| {
                let mut out = case.clone();
                *strings_mut(&mut out)[at] = t;
                visit(out)
            })?;
        }
        ControlFlow::Continue(())
    }

    fn mutate(&self, case: &PatternCase, g: &mut Source) -> PatternCase {
        let mut out = case.clone();
        match g.int(0u32..4) {
            0 => {
                let i = input(g, &out.spec);
                out.inputs.push(i);
            },
            1 if !out.inputs.is_empty() => {
                let at = g.index(out.inputs.len());
                out.inputs[at] = input(g, &out.spec);
            },
            2 => out.ignore_case = !out.ignore_case,
            _ => return self.generate(g),
        }
        out
    }
}

/// Numbers about a pattern case: how it is given, its inputs, and case
/// folding.
pub fn pattern_metrics(case: &PatternCase, _size: usize) -> Vec<(&'static str, f64)> {
    let b = |x: bool| f64::from(u8::from(x));
    vec![
        ("constructor string", b(matches!(case.spec, Spec::Text(_)))),
        ("with base", b(case.base.is_some())),
        ("inputs", case.inputs.len() as f64),
        ("ignore case", b(case.ignore_case)),
    ]
}

/// The generator of URLPattern cases.
#[ephyra::generator(metrics = pattern_metrics)]
pub fn pattern_cases() -> impl Gen<Value = PatternCase> {
    PatternCases
}
