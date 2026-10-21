//! URLPattern list cases: [`ListCase`] (pathname patterns and pathnames),
//! run by [`run`] through `ada::parse_url_pattern_list` and, route by route,
//! through `ada::url_pattern::exec`, and drawn by [`list_cases`].

use std::ops::ControlFlow;

use ephyra::{Gen, Source};

use crate::{gen, params::text};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ListCase {
    pub ignore_case: bool,
    pub patterns: Vec<String>,
    pub paths: Vec<String>,
}

/// One pathname: its canonical form, the list's answer, and each route's
/// answer alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathResult {
    pub canonical: String,
    /// The winning route's index, if any.
    pub route: Option<usize>,
    pub regexp: bool,
    pub truncated: bool,
    /// The winner's captures or regexp groups (`\0` for a group that did not
    /// participate).
    pub values: Vec<String>,
    /// Per route: `Some(values)` when it matches alone, its groups ordered
    /// by the list's group names.
    pub alone: Vec<Option<Vec<String>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListResult {
    pub list_ok: bool,
    /// Per pattern: whether it compiles alone.
    pub compiles: Vec<bool>,
    /// Per pathname that parses as a URL path (in order).
    pub paths: Vec<PathResult>,
}

extern "C" {
    fn ea_pattern_list(bytes: *const u8, len: usize, out: *mut *mut u8) -> usize;
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

/// Runs `case` in ada.
pub fn run(case: &ListCase) -> ListResult {
    let mut bytes = vec![u8::from(case.ignore_case)];
    for strings in [&case.patterns, &case.paths] {
        bytes.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        for s in strings {
            put_str(&mut bytes, s);
        }
    }
    let mut out = std::ptr::null_mut();
    // SAFETY: `bytes` is live for the call; the shim writes a malloc'd
    // buffer of the returned length to `out`, copied and then freed.
    let t = crate::ada::locked(|| unsafe {
        let len = ea_pattern_list(bytes.as_ptr(), bytes.len(), &mut out);
        let t = std::slice::from_raw_parts(out, len).to_vec();
        ea_free(out);
        t
    });
    let mut r = Reader(&t);
    let list_ok = r.byte() == 1;
    let compiles = case.patterns.iter().map(|_| r.byte() == 1).collect();
    let mut paths = Vec::new();
    if list_ok {
        for _ in &case.paths {
            if r.byte() == 0 {
                continue;
            }
            let canonical = r.str();
            let route = r.u32().checked_sub(1);
            let regexp = r.byte() == 1;
            let truncated = r.byte() == 1;
            let values = r.list();
            let alone = case
                .patterns
                .iter()
                .map(|_| {
                    let matched = r.byte() == 1;
                    let groups = r.list();
                    matched.then_some(groups)
                })
                .collect();
            paths.push(PathResult { canonical, route, regexp, truncated, values, alone });
        }
    }
    ListResult { list_ok, compiles, paths }
}

/// The kind sequence of a route in the plain subset: `/`-separated
/// segments, each a literal of unreserved characters (0), a `:name` (1), or
/// a final `*` (2). Other patterns are not in the subset.
pub fn plain_kinds(pattern: &str) -> Option<Vec<u8>> {
    let rest = pattern.strip_prefix('/')?;
    let segments: Vec<&str> = rest.split('/').collect();
    let mut kinds = Vec::new();
    for (i, s) in segments.iter().enumerate() {
        let kind = if *s == "*" && i + 1 == segments.len() {
            2
        } else if let Some(name) = s.strip_prefix(':') {
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_lowercase()) {
                return None;
            }
            1
        } else if !s.is_empty()
            && s.chars().all(|c| c.is_ascii_alphanumeric() || "-._~".contains(c))
        {
            0
        } else {
            return None;
        };
        kinds.push(kind);
    }
    Some(kinds)
}

const LITERALS: &[&str] =
    &["a", "b", "users", "api", "v1", "x.y", "A", "Users", "-", "~", "files", "1"];
const NAMES: &[&str] = &["id", "x", "y", "name"];

/// Parameter names for deep routes, beyond the fast path's eight captures.
const MANY_NAMES: &[&str] =
    &["pa", "pb", "pc", "pd", "pe", "pf", "pg", "ph", "pi", "pj", "pk", "pl"];

/// A route at the fast path's limits (`url_pattern_list_limits`): around
/// 16 pattern segments and 8 captures, mixing literals and parameters,
/// sometimes with a `*` tail.
fn deep_pattern(g: &mut Source) -> String {
    let n = *g.pick(&[7usize, 8, 9, 10, 15, 16, 17, 18]);
    let mut params = 0;
    let mut segs = Vec::new();
    for _ in 0..n {
        if params < MANY_NAMES.len() && g.bool() {
            segs.push(format!(":{}", MANY_NAMES[params]));
            params += 1;
        } else {
            segs.push(gen::pick_str(g, LITERALS));
        }
    }
    if g.bool_with(0.3) {
        segs.push("*".to_owned());
    }
    format!("/{}", segs.join("/"))
}

/// A pattern segment: mostly the plain subset, sometimes what the subset
/// leaves to the regex provider (custom regexp groups, modifiers, optional
/// groups, escapes, percent escapes).
fn pattern_segment(g: &mut Source, names: &mut Vec<&'static str>) -> String {
    let mut name = |g: &mut Source| {
        let free: Vec<&&str> = NAMES.iter().filter(|n| !names.contains(n)).collect();
        let n = if free.is_empty() { NAMES[0] } else { **g.pick(&free) };
        names.push(n);
        n
    };
    match g.int(0u32..20) {
        0..=8 => gen::pick_str(g, LITERALS),
        9..=13 => format!(":{}", name(g)),
        14 => {
            format!(":{}({})", name(g), g.pick(&["\\d+", "[a-z]+", "a|b", "[^/]+", ".*", "\\w*"]))
        },
        15 => format!(":{}{}", name(g), g.pick(&["?", "+", "*"])),
        16 => format!("{{{}}}?", gen::pick_str(g, LITERALS)),
        17 => format!("({})", g.pick(&["\\d+", "a|b", "[a-z]{2}", "x?"])),
        18 => gen::pick_str(g, &["%20", "%41", "caf%C3%A9", "a%2Fb", "\\:", "a\\*b", "*.png"]),
        _ => "*".to_owned(),
    }
}

fn pattern(g: &mut Source) -> String {
    let mut names = Vec::new();
    let n = gen::few(g, 5);
    let mut out = String::new();
    for _ in 0..n {
        out.push('/');
        out.push_str(&pattern_segment(g, &mut names));
    }
    if n == 0 || g.bool_with(0.15) {
        out.push('/');
    }
    if g.bool_with(0.15) {
        out.push_str("/*");
    }
    out
}

/// A route derived from `from`, as route sets are: a literal segment made a
/// parameter or a parameter made a literal, the route cut short and given a
/// `*` tail, or a segment appended.
fn derive(g: &mut Source, from: &str) -> String {
    let mut segs: Vec<String> = from.split('/').skip(1).map(str::to_owned).collect();
    if segs.last().is_some_and(|s| s.is_empty()) {
        segs.pop();
    }
    let used: Vec<&str> = segs.iter().filter_map(|s| s.strip_prefix(':')).collect();
    let free: Vec<&&str> =
        NAMES.iter().filter(|n| !used.iter().any(|u| u.starts_with(**n))).collect();
    match g.int(0u32..4) {
        0 if !segs.is_empty() && !free.is_empty() => {
            let at = g.index(segs.len());
            segs[at] = format!(":{}", g.pick(&free));
        },
        1 if !segs.is_empty() => {
            let at = g.index(segs.len());
            segs[at] = gen::pick_str(g, LITERALS);
        },
        2 => {
            let keep = g.int(0..=segs.len());
            segs.truncate(keep);
            segs.push("*".to_owned());
        },
        _ => {
            if segs.last().is_some_and(|s| s == "*") {
                segs.pop();
            }
            segs.push(gen::pick_str(g, LITERALS));
        },
    }
    format!("/{}", segs.join("/"))
}

/// A pathname segment standing for a pattern segment: a literal kept (or
/// recased), a parameter given a value, a wildcard given a tail of any
/// segments, an optional part kept or dropped.
fn instantiate(g: &mut Source, pattern: &str) -> String {
    let mut out = String::new();
    for seg in pattern.split('/').skip(1) {
        let value = if seg == "*" {
            // Around the fast path's 24 input segments, now and then.
            let n = if g.bool_with(0.15) { g.int(18usize..=30) } else { gen::few(g, 3) };
            (0..n).map(|_| gen::pick_str(g, LITERALS)).collect::<Vec<_>>().join("/")
        } else if seg.starts_with(':') && g.bool_with(0.03) {
            // Around the fast path's 4096 input bytes.
            "a".repeat(*g.pick(&[2040usize, 4080, 4090, 4100, 5000]))
        } else if seg.starts_with(':') || seg.starts_with('(') {
            gen::pick_str(
                g,
                &["1", "42", "abc", "a", "b", "x.y", "%20", "caf%C3%A9", "AB", "-", "ab"],
            )
        } else if seg.starts_with('{') {
            if g.bool() {
                seg.trim_start_matches('{').trim_end_matches("}?").to_owned()
            } else {
                continue;
            }
        } else if g.bool_with(0.2) {
            seg.to_ascii_uppercase()
        } else {
            seg.to_owned()
        };
        out.push('/');
        out.push_str(&value);
    }
    if out.is_empty() {
        out.push('/');
    }
    out
}

fn path(g: &mut Source, patterns: &[String]) -> String {
    if !patterns.is_empty() && g.bool_with(0.7) {
        let p = patterns[g.index(patterns.len())].clone();
        return instantiate(g, &p);
    }
    let mut out = String::new();
    for _ in 0..gen::few(g, 6) {
        out.push('/');
        out.push_str(&gen::pick_str(
            g,
            &["a", "b", "users", "1", "", "x.y", "%2F", "A", "..", "."],
        ));
    }
    if out.is_empty() {
        out.push('/');
    }
    out
}

/// `n` distinct one-character segments (at most 65): a wide fan-out that
/// the first byte alone tells apart.
fn single_chars(n: usize) -> Vec<String> {
    let chars = "abcdefghijklmnopqrstuvwxyz0123456789-_.ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    (0..n.min(chars.len())).map(|i| chars[i..i + 1].to_owned()).collect()
}

/// `n` (or fewer) distinct segments that no single byte position tells
/// apart, so the compiler's witness search must combine positions or give
/// up and demote the node: every word of one length over a two-letter
/// alphabet (needs two or three positions); one long word varied only in
/// its middle, more than eight bytes from either end (outside every
/// witness position); or words of several lengths sharing their first and
/// last byte (only the length tells them apart).
fn colliding_words(g: &mut Source, n: usize) -> Vec<String> {
    let mut out: Vec<String> = match g.int(0u32..3) {
        0 => {
            let len = *g.pick(&[3u32, 4, 5]);
            let (a, b) = *g.pick(&[('a', 'b'), ('x', 'X'), ('0', '1')]);
            (0..1u32 << len)
                .map(|bits| (0..len).map(|i| if bits >> i & 1 == 1 { b } else { a }).collect())
                .collect()
        },
        1 => {
            let edge = "e".repeat(*g.pick(&[8usize, 9, 12]));
            (0..n).map(|i| format!("{edge}{}{edge}", single_chars(65)[i % 65])).collect()
        },
        _ => (0..n).map(|i| format!("a{}z", "m".repeat(i))).collect(),
    };
    out.truncate(n);
    out
}

/// A pathname one byte away from a route's (its last character replaced):
/// it lands on the same projection slot as the route without matching it.
fn near_miss(g: &mut Source, patterns: &[String]) -> String {
    let at = g.index(patterns.len());
    let mut p = instantiate(g, &patterns[at]);
    if let Some(c) = p.pop() {
        p.push(if c == 'a' { 'b' } else { 'a' });
    }
    p
}

/// URLPattern list cases: see the module documentation. Shrinking drops
/// patterns and pathnames, then deletes characters; mutation redraws or
/// adds a pattern or a pathname, or flips `ignore_case`.
pub struct ListCases;

impl Gen for ListCases {
    type Value = ListCase;

    fn generate(&self, g: &mut Source) -> ListCase {
        // A wide route set: many static routes sharing a prefix and differing
        // in one segment, so a trie node has a large fan-out (the first-byte
        // index, perfect-hash projection and witness/multiplier search in the
        // compiler). Distinct single characters, so the fan-out is real.
        if g.bool_with(0.12) {
            let prefix = if g.bool() { "/api".to_owned() } else { String::new() };
            let n = *g.pick(&[8usize, 12, 20, 32, 48]);
            let children = if g.bool() { single_chars(n) } else { colliding_words(g, n) };
            let mut patterns: Vec<String> =
                children.iter().map(|c| format!("{prefix}/{c}")).collect();
            if g.bool_with(0.5) {
                let at = g.index(patterns.len());
                patterns[at].push_str(if g.bool() { "/:id" } else { "/*" });
            }
            let paths = (0..1 + gen::few(g, 4))
                .map(|_| if g.bool_with(0.3) { near_miss(g, &patterns) } else { path(g, &patterns) })
                .collect();
            return ListCase { ignore_case: g.bool_with(0.2), patterns, paths };
        }
        let max = gen::sized(g, 20).min(8);
        let first = if g.bool_with(0.1) { deep_pattern(g) } else { pattern(g) };
        let mut patterns: Vec<String> = vec![first];
        for _ in 0..gen::few(g, max) {
            let p = if g.bool_with(0.6) {
                let from = patterns[g.index(patterns.len())].clone();
                derive(g, &from)
            } else {
                pattern(g)
            };
            patterns.push(p);
        }
        let paths = (0..1 + gen::few(g, 4)).map(|_| path(g, &patterns)).collect();
        ListCase { ignore_case: g.bool_with(0.2), patterns, paths }
    }

    fn shrink(
        &self,
        case: &ListCase,
        visit: &mut dyn FnMut(ListCase) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        for k in (0..case.patterns.len()).rev() {
            let mut out = case.clone();
            out.patterns.remove(k);
            visit(out)?;
        }
        for k in (0..case.paths.len()).rev() {
            let mut out = case.clone();
            out.paths.remove(k);
            visit(out)?;
        }
        if case.ignore_case {
            visit(ListCase { ignore_case: false, ..case.clone() })?;
        }
        for k in 0..case.patterns.len() {
            gen::deletions(&case.patterns[k], &mut |t| {
                let mut out = case.clone();
                out.patterns[k] = t;
                visit(out)
            })?;
        }
        for k in 0..case.paths.len() {
            gen::deletions(&case.paths[k], &mut |t| {
                let mut out = case.clone();
                out.paths[k] = t;
                visit(out)
            })?;
        }
        ControlFlow::Continue(())
    }

    fn mutate(&self, case: &ListCase, g: &mut Source) -> ListCase {
        let mut out = case.clone();
        match g.int(0u32..5) {
            0 => {
                let at = g.int(0..=out.patterns.len());
                let p = if out.patterns.is_empty() || g.bool() {
                    pattern(g)
                } else {
                    let from = out.patterns[g.index(out.patterns.len())].clone();
                    derive(g, &from)
                };
                out.patterns.insert(at, p);
            },
            1 if !out.patterns.is_empty() => {
                let at = g.index(out.patterns.len());
                out.patterns[at] = pattern(g);
            },
            2 => {
                let p = path(g, &out.patterns);
                out.paths.push(p);
            },
            3 if !out.paths.is_empty() => {
                let at = g.index(out.paths.len());
                out.paths[at] = path(g, &out.patterns);
            },
            _ => out.ignore_case = !out.ignore_case,
        }
        out
    }
}

/// Numbers about a list case: routes, pathnames, routes in the plain
/// subset, and whether case is ignored.
pub fn list_metrics(case: &ListCase, _size: usize) -> Vec<(&'static str, f64)> {
    vec![
        ("patterns", case.patterns.len() as f64),
        ("paths", case.paths.len() as f64),
        (
            "plain patterns",
            case.patterns.iter().filter(|p| plain_kinds(p).is_some()).count() as f64,
        ),
        ("ignore case", f64::from(u8::from(case.ignore_case))),
    ]
}

/// The generator of URLPattern list cases.
#[ephyra::generator(metrics = list_metrics)]
pub fn list_cases() -> impl Gen<Value = ListCase> {
    ListCases
}
